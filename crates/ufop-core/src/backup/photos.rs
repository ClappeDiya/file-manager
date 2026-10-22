//! macOS Photos library export as a backup source.
//!
//! A job source `photos://<path to .photoslibrary>` is expanded by the
//! scanner into the library's original files under stable logical paths:
//!
//! ```text
//! <parent>/<library name> (Photos export)/Photos/<YYYY>/<MM>/<original name>
//! <parent>/<library name> (Photos export)/albums.json
//! ```
//!
//! `albums.json` ([`Manifest`]) records each asset's UUID, creation date,
//! favourite / hidden flags and albums, so the albums can be rebuilt after a
//! restore, which itself produces a plain folder tree.
//!
//! The library database (`database/Photos.sqlite`) is never opened in place:
//! Photos keeps it locked, so it is copied (with its `-wal` / `-shm` files)
//! into a private temporary folder and queried there. The schema differs
//! between macOS versions; missing tables or columns give fewer fields (and a
//! warning), not a failure. Nothing here is macOS-specific, so the reader is
//! built and tested on every platform.

use crate::error::AppError;
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

/// Source URI scheme: `photos:///Users/me/Pictures/Photos Library.photoslibrary`.
pub const SOURCE_SCHEME: &str = "photos://";
/// Name of the metadata file written at the export root.
pub const MANIFEST_NAME: &str = "albums.json";
/// Folder below the export root holding the originals.
pub const PHOTOS_DIR: &str = "Photos";
/// Folder for assets without a creation date.
pub const UNDATED_DIR: &str = "Undated";
/// `format` value of [`Manifest`].
pub const MANIFEST_FORMAT: &str = "ufop-photos-export";
/// Core Data timestamps count seconds from 2001-01-01T00:00:00Z.
const CORE_DATA_EPOCH: i64 = 978_307_200;
/// `ZGENERICALBUM.ZKIND` of albums the user created.
const USER_ALBUM_KIND: i64 = 2;

/// What a Photos source exports. Applies to every Photos source of a job.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PhotosOptions {
    /// Include assets in the Hidden album.
    pub include_hidden: bool,
    /// Include assets in Recently Deleted.
    pub include_trashed: bool,
    /// Only assets in these user albums (titles, case-insensitive); empty = all.
    pub albums: Vec<String>,
}

/// Library path of a `photos://` source, or `None` for an ordinary path.
///
/// Accepts `photos:///abs/path`, `photos://C:\path`, `photos:///C:/path` and
/// percent-encoded characters (`%20`).
pub fn parse_source(source: &str) -> Option<PathBuf> {
    let rest = source.trim().strip_prefix(SOURCE_SCHEME)?;
    let rest = percent_decode(rest);
    // `photos:///C:/x` → `C:/x`
    let b = rest.as_bytes();
    let rest = if b.len() >= 3 && b[0] == b'/' && b[1].is_ascii_alphabetic() && b[2] == b':' { rest[1..].to_string() } else { rest };
    let rest = rest.trim_end_matches(['/', '\\']);
    if rest.is_empty() {
        return None;
    }
    Some(PathBuf::from(rest))
}

/// Whether a job source string names a Photos library.
pub fn is_photos_source(source: &str) -> bool {
    source.trim().starts_with(SOURCE_SCHEME)
}

/// `photos://` URI for a library path.
pub fn source_uri(library: &Path) -> String {
    let s = library.to_string_lossy();
    if s.starts_with('/') {
        format!("{SOURCE_SCHEME}{s}")
    } else {
        format!("{SOURCE_SCHEME}/{s}")
    }
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = ((b[i + 1] as char).to_digit(16), (b[i + 2] as char).to_digit(16)) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

/// Logical root of a library's export: a sibling of the library named
/// `<library name> (Photos export)`, so a restore to the original location
/// never writes into the library bundle.
pub fn export_root(library: &Path) -> PathBuf {
    let stem = library.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Photos Library".into());
    let name = format!("{stem} (Photos export)");
    match library.parent() {
        Some(p) => p.join(name),
        None => PathBuf::from(name),
    }
}

/// A Photos library found on this computer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryCandidate {
    pub path: String,
    pub name: String,
    /// `photos://` source for the job editor.
    pub source: String,
    /// The library has `database/Photos.sqlite` (Photos 5+ / macOS 10.15+, or
    /// an upgraded older library).
    pub has_database: bool,
    /// `~/Pictures/Photos Library.photoslibrary`, the usual system library.
    pub is_default: bool,
}

/// `.photoslibrary` bundles directly inside `dir` (e.g. `~/Pictures`).
pub fn find_libraries(dir: &Path) -> Vec<LibraryCandidate> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<LibraryCandidate> = rd
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir() && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("photoslibrary")))
        .map(|p| LibraryCandidate {
            name: p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
            source: source_uri(&p),
            has_database: database_path(&p).is_file(),
            is_default: p.file_name().is_some_and(|n| n == "Photos Library.photoslibrary"),
            path: p.to_string_lossy().into_owned(),
        })
        .collect();
    out.sort_by(|a, b| b.is_default.cmp(&a.is_default).then_with(|| a.name.cmp(&b.name)));
    out
}

fn database_path(library: &Path) -> PathBuf {
    library.join("database").join("Photos.sqlite")
}

/// One asset row of `ZASSET` (or `ZGENERICASSET` on older systems).
#[derive(Debug, Clone, PartialEq)]
pub struct PhotosAsset {
    pub pk: i64,
    pub uuid: String,
    pub directory: Option<String>,
    pub filename: String,
    pub original_filename: Option<String>,
    /// Unix seconds (UTC).
    pub created: Option<i64>,
    /// Offset of the capture time zone from UTC in seconds, when recorded.
    pub tz_offset: Option<i64>,
    pub trashed: bool,
    pub hidden: bool,
    pub favorite: bool,
    /// `ZKIND`: 0 photo, 1 video.
    pub kind: Option<i64>,
    pub uti: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PhotosAlbum {
    pub pk: i64,
    pub uuid: Option<String>,
    pub title: String,
    /// Asset primary keys in album order.
    pub assets: Vec<i64>,
}

/// Everything read from a library database.
#[derive(Debug, Clone)]
pub struct PhotosLibrary {
    pub path: PathBuf,
    pub assets: Vec<PhotosAsset>,
    /// User albums (not in Recently Deleted).
    pub albums: Vec<PhotosAlbum>,
    /// Schema features that were missing.
    pub warnings: Vec<String>,
}

/// Private copy of the database, removed on drop.
struct TempCopy(PathBuf);

impl Drop for TempCopy {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

impl PhotosLibrary {
    /// Read the library at `path`, copying its database into `temp_root` first.
    pub fn open(path: &Path, temp_root: &Path) -> Result<Self, AppError> {
        if !path.is_dir() {
            return Err(AppError::file_op(
                format!("Photos library {} does not exist", path.display()),
                "Choose the .photoslibrary bundle (usually ~/Pictures/Photos Library.photoslibrary).",
            ));
        }
        let db = database_path(path);
        if !db.is_file() {
            return Err(AppError::file_op(
                format!("{} has no database/Photos.sqlite", path.display()),
                "Open the library once in Photos on macOS 10.15 or later so it is upgraded, then try again.",
            ));
        }
        let tmp = TempCopy(temp_root.join(format!("ufop-photos-{}", uuid::Uuid::new_v4().simple())));
        std::fs::create_dir_all(&tmp.0)?;
        let copy = tmp.0.join("Photos.sqlite");
        std::fs::copy(&db, &copy).map_err(|e| {
            AppError::file_op(
                format!("Cannot read {}: {e}", db.display()),
                "Grant UFOP Full Disk Access (System Settings › Privacy & Security) so it can read the Photos library.",
            )
        })?;
        for ext in ["-wal", "-shm"] {
            let side = PathBuf::from(format!("{}{ext}", db.to_string_lossy()));
            if side.is_file() {
                std::fs::copy(&side, tmp.0.join(format!("Photos.sqlite{ext}")))?;
            }
        }
        // Read-write on the private copy so SQLite can replay the WAL;
        // `query_only` keeps this code from ever changing it.
        let conn = Connection::open_with_flags(&copy, OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX)
            .map_err(|e| AppError::database(format!("Cannot open the Photos database: {e}"), "The library may be damaged; open it in Photos to repair it."))?;
        conn.execute_batch("PRAGMA query_only = ON;")?;
        let (assets, albums, warnings) = read_database(&conn).map_err(|e| match e {
            AppError::Database { message, advice } => AppError::Database { message: format!("Photos database {}: {message}", path.display()), advice },
            other => other,
        })?;
        drop(conn);
        Ok(Self { path: path.to_path_buf(), assets, albums, warnings })
    }
}

fn table_exists(conn: &Connection, name: &str) -> Result<bool, AppError> {
    Ok(conn.query_row("SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = ?1", [name], |r| r.get::<_, i64>(0))? > 0)
}

fn columns(conn: &Connection, table: &str) -> Result<HashSet<String>, AppError> {
    let mut st = conn.prepare(&format!("PRAGMA table_info(\"{}\")", table.replace('"', "")))?;
    let cols = st.query_map([], |r| r.get::<_, String>(1))?.collect::<Result<Vec<_>, _>>()?;
    Ok(cols.into_iter().map(|c| c.to_ascii_uppercase()).collect())
}

/// `col` when the table has it, else `NULL` (so the query still runs).
fn col_or_null(cols: &HashSet<String>, col: &str) -> String {
    if cols.contains(col) {
        col.to_string()
    } else {
        "NULL".to_string()
    }
}

/// Integer/real/text cell as i64 (Core Data stores booleans as integers).
fn int_cell(v: rusqlite::types::ValueRef<'_>) -> Option<i64> {
    use rusqlite::types::ValueRef;
    match v {
        ValueRef::Integer(i) => Some(i),
        ValueRef::Real(f) => Some(f as i64),
        ValueRef::Text(t) => std::str::from_utf8(t).ok().and_then(|s| s.trim().parse().ok()),
        _ => None,
    }
}

fn real_cell(v: rusqlite::types::ValueRef<'_>) -> Option<f64> {
    use rusqlite::types::ValueRef;
    match v {
        ValueRef::Integer(i) => Some(i as f64),
        ValueRef::Real(f) => Some(f),
        ValueRef::Text(t) => std::str::from_utf8(t).ok().and_then(|s| s.trim().parse().ok()),
        _ => None,
    }
}

fn text_cell(v: rusqlite::types::ValueRef<'_>) -> Option<String> {
    use rusqlite::types::ValueRef;
    match v {
        ValueRef::Text(t) => Some(String::from_utf8_lossy(t).into_owned()).filter(|s| !s.is_empty()),
        ValueRef::Integer(i) => Some(i.to_string()),
        _ => None,
    }
}

type DbContent = (Vec<PhotosAsset>, Vec<PhotosAlbum>, Vec<String>);

fn read_database(conn: &Connection) -> Result<DbContent, AppError> {
    let mut warnings = Vec::new();
    let table = if table_exists(conn, "ZASSET")? {
        "ZASSET"
    } else if table_exists(conn, "ZGENERICASSET")? {
        "ZGENERICASSET"
    } else {
        return Err(AppError::database("no asset table (ZASSET) found", "This does not look like a Photos library made by macOS 10.15 or later."));
    };
    let cols = columns(conn, table)?;
    if !cols.contains("Z_PK") || !cols.contains("ZFILENAME") {
        return Err(AppError::database(format!("{table} has no Z_PK / ZFILENAME column"), "This Photos library format is not supported."));
    }
    let wanted = ["ZUUID", "ZDIRECTORY", "ZDATECREATED", "ZTRASHEDSTATE", "ZHIDDEN", "ZFAVORITE", "ZKIND", "ZUNIFORMTYPEIDENTIFIER"];
    for c in wanted {
        if !cols.contains(c) {
            warnings.push(format!("Photos: the asset table has no {c} column; that information is not exported"));
        }
    }
    let sel: Vec<String> = wanted.iter().map(|c| col_or_null(&cols, c)).collect();
    let sql = format!("SELECT Z_PK, ZFILENAME, {} FROM {table} ORDER BY Z_PK", sel.join(", "));
    let mut st = conn.prepare(&sql)?;
    let mut rows = st.query([])?;
    let mut assets = Vec::new();
    while let Some(r) = rows.next()? {
        let pk: i64 = r.get(0)?;
        let Some(filename) = text_cell(r.get_ref(1)?) else { continue };
        let flag = |i: usize| -> Result<bool, AppError> { Ok(int_cell(r.get_ref(i)?).unwrap_or(0) != 0) };
        assets.push(PhotosAsset {
            pk,
            filename,
            uuid: text_cell(r.get_ref(2)?).unwrap_or_else(|| format!("pk-{pk}")),
            directory: text_cell(r.get_ref(3)?),
            created: real_cell(r.get_ref(4)?).filter(|f| f.is_finite()).map(|f| f.floor() as i64 + CORE_DATA_EPOCH),
            trashed: flag(5)?,
            hidden: flag(6)?,
            favorite: flag(7)?,
            kind: int_cell(r.get_ref(8)?),
            uti: text_cell(r.get_ref(9)?),
            original_filename: None,
            tz_offset: None,
        });
    }
    drop(rows);
    drop(st);

    // Original file names and capture time zones.
    if table_exists(conn, "ZADDITIONALASSETATTRIBUTES")? {
        let ac = columns(conn, "ZADDITIONALASSETATTRIBUTES")?;
        if ac.contains("ZASSET") {
            let sql = format!(
                "SELECT ZASSET, {}, {} FROM ZADDITIONALASSETATTRIBUTES WHERE ZASSET IS NOT NULL",
                col_or_null(&ac, "ZORIGINALFILENAME"),
                col_or_null(&ac, "ZTIMEZONEOFFSET")
            );
            let mut st = conn.prepare(&sql)?;
            let mut rows = st.query([])?;
            let mut extra: HashMap<i64, (Option<String>, Option<i64>)> = HashMap::new();
            while let Some(r) = rows.next()? {
                if let Some(pk) = int_cell(r.get_ref(0)?) {
                    extra.insert(pk, (text_cell(r.get_ref(1)?), int_cell(r.get_ref(2)?)));
                }
            }
            for a in &mut assets {
                if let Some((name, tz)) = extra.remove(&a.pk) {
                    a.original_filename = name;
                    a.tz_offset = tz.filter(|t| t.abs() <= 18 * 3600);
                }
            }
            if !ac.contains("ZORIGINALFILENAME") {
                warnings.push("Photos: original file names are not recorded in this library; internal names are used".into());
            }
        } else {
            warnings.push("Photos: unrecognised additional-attributes table; internal file names are used".into());
        }
    } else {
        warnings.push("Photos: no additional-attributes table; internal file names and UTC dates are used".into());
    }

    let albums = read_albums(conn, &mut warnings)?;
    Ok((assets, albums, warnings))
}

/// The album ↔ asset join table: `Z_<n>ASSETS` with a `Z_<n>ALBUMS` and a
/// `Z_<m>ASSETS` column (the numbers change between macOS versions).
fn find_album_join(conn: &Connection) -> Result<Option<(String, String, String, Option<String>)>, AppError> {
    let mut st = conn.prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name LIKE 'Z\\_%ASSETS' ESCAPE '\\' ORDER BY name")?;
    let names = st.query_map([], |r| r.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
    let numbered = |c: &str, suffix: &str| -> bool {
        c.strip_prefix("Z_").and_then(|r| r.strip_suffix(suffix)).is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
    };
    for name in names {
        if !numbered(&name.to_ascii_uppercase(), "ASSETS") {
            continue;
        }
        let cols = columns(conn, &name)?;
        let album = cols.iter().find(|c| numbered(c, "ALBUMS")).cloned();
        let asset = cols.iter().find(|c| numbered(c, "ASSETS")).cloned();
        if let (Some(album), Some(asset)) = (album, asset) {
            let order = cols.iter().find(|c| c.starts_with("Z_FOK_") && c.ends_with("ASSETS")).cloned();
            return Ok(Some((name, album, asset, order)));
        }
    }
    Ok(None)
}

fn read_albums(conn: &Connection, warnings: &mut Vec<String>) -> Result<Vec<PhotosAlbum>, AppError> {
    if !table_exists(conn, "ZGENERICALBUM")? {
        warnings.push("Photos: no album table (ZGENERICALBUM); albums are not exported".into());
        return Ok(Vec::new());
    }
    let ac = columns(conn, "ZGENERICALBUM")?;
    if !ac.contains("Z_PK") || !ac.contains("ZTITLE") {
        warnings.push("Photos: the album table has no titles; albums are not exported".into());
        return Ok(Vec::new());
    }
    let kind = if ac.contains("ZKIND") { format!("ZKIND = {USER_ALBUM_KIND}") } else { "1".into() };
    let trashed = if ac.contains("ZTRASHEDSTATE") { "COALESCE(ZTRASHEDSTATE, 0) = 0" } else { "1" };
    let sql = format!("SELECT Z_PK, ZTITLE, {} FROM ZGENERICALBUM WHERE {kind} AND {trashed} AND ZTITLE IS NOT NULL ORDER BY Z_PK", col_or_null(&ac, "ZUUID"));
    let mut st = conn.prepare(&sql)?;
    let mut albums: Vec<PhotosAlbum> = st
        .query_map([], |r| Ok(PhotosAlbum { pk: r.get(0)?, title: r.get::<_, String>(1)?, uuid: text_cell(r.get_ref(2)?), assets: Vec::new() }))?
        .collect::<Result<Vec<_>, _>>()?;
    drop(st);
    if albums.is_empty() {
        return Ok(albums);
    }
    let Some((table, album_col, asset_col, order_col)) = find_album_join(conn)? else {
        warnings.push("Photos: the album membership table was not found; album contents are not exported".into());
        return Ok(albums);
    };
    let order = order_col.map(|c| format!("{c}, ")).unwrap_or_default();
    let sql = format!("SELECT \"{album_col}\", \"{asset_col}\" FROM \"{table}\" ORDER BY \"{album_col}\", {order}\"{asset_col}\"");
    let mut st = conn.prepare(&sql)?;
    let mut rows = st.query([])?;
    let index: HashMap<i64, usize> = albums.iter().enumerate().map(|(i, a)| (a.pk, i)).collect();
    while let Some(r) = rows.next()? {
        if let (Some(al), Some(asset)) = (int_cell(r.get_ref(0)?), int_cell(r.get_ref(1)?)) {
            if let Some(&i) = index.get(&al) {
                albums[i].assets.push(asset);
            }
        }
    }
    Ok(albums)
}

/// One original to back up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportItem {
    /// Path recorded in the backup.
    pub logical: PathBuf,
    /// The original inside the library.
    pub original: PathBuf,
    pub uuid: String,
}

/// Counts of what an export left out.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportStats {
    pub exported: u64,
    pub skipped_hidden: u64,
    pub skipped_trashed: u64,
    pub skipped_album_filter: u64,
    pub missing_originals: u64,
}

/// A library expanded into backup items plus its metadata file.
#[derive(Debug, Clone)]
pub struct PhotosExport {
    pub root: PathBuf,
    pub items: Vec<ExportItem>,
    /// Serialized [`Manifest`] (deterministic for unchanged libraries).
    pub manifest: Vec<u8>,
    pub stats: ExportStats,
    pub warnings: Vec<String>,
}

impl PhotosExport {
    /// Logical path of `albums.json`.
    pub fn manifest_path(&self) -> PathBuf {
        self.root.join(MANIFEST_NAME)
    }
}

/// `albums.json`: everything needed to rebuild albums after a restore.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub format: String,
    pub version: u32,
    pub library_name: String,
    pub albums: Vec<ManifestAlbum>,
    pub assets: Vec<ManifestAsset>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestAlbum {
    pub title: String,
    pub uuid: Option<String>,
    /// Asset paths relative to the export root (`Photos/2024/03/IMG_0001.HEIC`).
    pub assets: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestAsset {
    /// Relative to the export root, `/`-separated.
    pub path: String,
    pub uuid: String,
    pub original_filename: Option<String>,
    /// Capture time, RFC 3339 with the recorded time-zone offset.
    pub created: Option<String>,
    pub favorite: bool,
    pub hidden: bool,
    pub trashed: bool,
    /// `photo`, `video` or `other`.
    pub kind: String,
    pub uniform_type_identifier: Option<String>,
    /// Titles of the user albums containing this asset.
    pub albums: Vec<String>,
}

/// Characters that are unsafe in a file name on some platform.
fn safe_name(s: &str) -> String {
    let cleaned: String = s.chars().map(|c| if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '_' } else { c }).collect();
    let trimmed = cleaned.trim().trim_end_matches(['.', ' ']);
    if trimmed.is_empty() || trimmed == "." || trimmed == ".." {
        String::new()
    } else {
        trimmed.to_string()
    }
}

/// A single relative path segment list without `..`, roots or prefixes.
fn relative_components(s: &str) -> Option<PathBuf> {
    let p = Path::new(s);
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::Normal(n) => out.push(n),
            Component::CurDir => {}
            _ => return None,
        }
    }
    Some(out)
}

/// Where the original of `a` lives: `originals/` (Photos 5+) or `Masters/`
/// (libraries upgraded from Photos 4 and earlier).
pub fn locate_original(library: &Path, a: &PhotosAsset) -> Option<PathBuf> {
    let file = relative_components(&a.filename).filter(|f| f.components().count() == 1)?;
    let dir = match a.directory.as_deref() {
        Some(d) => relative_components(d)?,
        None => PathBuf::new(),
    };
    ["originals", "Masters"].iter().map(|base| library.join(base).join(&dir).join(&file)).find(|p| p.is_file())
}

fn split_ext(name: &str) -> (&str, Option<&str>) {
    match name.rfind('.') {
        Some(i) if i > 0 && i + 1 < name.len() => (&name[..i], Some(&name[i + 1..])),
        _ => (name, None),
    }
}

/// File name used in the export: the original import name, else the
/// internal one; the internal extension is added when the original has none.
fn display_name(a: &PhotosAsset) -> String {
    let internal = safe_name(&a.filename);
    let name = a.original_filename.as_deref().map(safe_name).filter(|n| !n.is_empty()).unwrap_or_else(|| internal.clone());
    let name = if name.is_empty() { safe_name(&a.uuid) } else { name };
    match (split_ext(&name).1, split_ext(&internal).1) {
        (None, Some(ext)) => format!("{name}.{ext}"),
        _ => name,
    }
}

fn date_folder(a: &PhotosAsset) -> PathBuf {
    let local = a.created.map(|c| c + a.tz_offset.unwrap_or(0));
    match local.and_then(|t| chrono::DateTime::from_timestamp(t, 0)) {
        Some(d) => PathBuf::from(PHOTOS_DIR).join(d.format("%Y").to_string()).join(d.format("%m").to_string()),
        None => PathBuf::from(PHOTOS_DIR).join(UNDATED_DIR),
    }
}

fn created_rfc3339(a: &PhotosAsset) -> Option<String> {
    let t = a.created?;
    let off = chrono::FixedOffset::east_opt(a.tz_offset.unwrap_or(0) as i32).unwrap_or_else(|| chrono::FixedOffset::east_opt(0).expect("zero offset"));
    chrono::DateTime::from_timestamp(t, 0).map(|d| d.with_timezone(&off).to_rfc3339())
}

fn rel_string(p: &Path) -> String {
    p.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/")
}

/// Choose which assets to export, give them collision-free logical paths and
/// build the metadata manifest. Deterministic for an unchanged library.
pub fn plan_export(lib: &PhotosLibrary, opts: &PhotosOptions) -> PhotosExport {
    let root = export_root(&lib.path);
    let mut warnings: Vec<String> = lib.warnings.clone();
    let mut stats = ExportStats::default();

    // Album filter.
    let wanted: Vec<String> = opts.albums.iter().map(|t| t.trim().to_lowercase()).filter(|t| !t.is_empty()).collect();
    let albums: Vec<&PhotosAlbum> = if wanted.is_empty() {
        lib.albums.iter().collect()
    } else {
        for w in &wanted {
            if !lib.albums.iter().any(|a| a.title.trim().to_lowercase() == *w) {
                warnings.push(format!("Photos: no album named '{w}' in {}", lib.path.display()));
            }
        }
        lib.albums.iter().filter(|a| wanted.contains(&a.title.trim().to_lowercase())).collect()
    };
    let in_selected: HashSet<i64> = albums.iter().flat_map(|a| a.assets.iter().copied()).collect();
    let mut asset_albums: HashMap<i64, Vec<String>> = HashMap::new();
    for al in &lib.albums {
        for pk in &al.assets {
            let v = asset_albums.entry(*pk).or_default();
            if !v.contains(&al.title) {
                v.push(al.title.clone());
            }
        }
    }

    // Selection; sorted so name collisions resolve the same way every run.
    let mut chosen: Vec<(&PhotosAsset, PathBuf)> = Vec::new();
    let mut ordered: Vec<&PhotosAsset> = lib.assets.iter().collect();
    ordered.sort_by(|a, b| a.created.unwrap_or(i64::MAX).cmp(&b.created.unwrap_or(i64::MAX)).then_with(|| a.uuid.cmp(&b.uuid)).then_with(|| a.pk.cmp(&b.pk)));
    for a in ordered {
        if a.trashed && !opts.include_trashed {
            stats.skipped_trashed += 1;
            continue;
        }
        if a.hidden && !opts.include_hidden {
            stats.skipped_hidden += 1;
            continue;
        }
        if !wanted.is_empty() && !in_selected.contains(&a.pk) {
            stats.skipped_album_filter += 1;
            continue;
        }
        match locate_original(&lib.path, a) {
            Some(p) => chosen.push((a, p)),
            None => {
                stats.missing_originals += 1;
                warnings.push(format!(
                    "Photos: the original of '{}' ({}) is not in the library (it may be kept only in iCloud with \"Optimize Mac Storage\"); skipped",
                    display_name(a),
                    a.uuid
                ));
            }
        }
    }

    // Collision-free names (case-insensitive, as on APFS / NTFS defaults).
    let mut taken: HashSet<String> = HashSet::new();
    let mut items = Vec::new();
    let mut by_pk: HashMap<i64, String> = HashMap::new();
    let mut manifest_assets: BTreeMap<String, ManifestAsset> = BTreeMap::new();
    for (a, original) in chosen {
        let folder = date_folder(a);
        let name = display_name(a);
        let (stem, ext) = split_ext(&name);
        let short: String = a.uuid.chars().filter(|c| c.is_ascii_alphanumeric()).take(8).collect();
        let with = |tag: &str| match ext {
            Some(e) => format!("{stem} ({tag}).{e}"),
            None => format!("{stem} ({tag})"),
        };
        let mut candidates = vec![name.clone(), with(&short), with(&safe_name(&a.uuid))];
        candidates.extend((2..).take(1000).map(|n| with(&format!("{short}-{n}"))));
        let rel = candidates
            .into_iter()
            .map(|n| folder.join(n))
            .find(|p| taken.insert(rel_string(p).to_lowercase()))
            .unwrap_or_else(|| folder.join(format!("{}-{}", a.pk, name)));
        let rel_s = rel_string(&rel);
        by_pk.insert(a.pk, rel_s.clone());
        manifest_assets.insert(
            rel_s.clone(),
            ManifestAsset {
                path: rel_s,
                uuid: a.uuid.clone(),
                original_filename: a.original_filename.clone(),
                created: created_rfc3339(a),
                favorite: a.favorite,
                hidden: a.hidden,
                trashed: a.trashed,
                kind: match a.kind {
                    Some(0) => "photo",
                    Some(1) => "video",
                    _ => "other",
                }
                .into(),
                uniform_type_identifier: a.uti.clone(),
                albums: asset_albums.get(&a.pk).cloned().unwrap_or_default(),
            },
        );
        items.push(ExportItem { logical: root.join(&rel), original, uuid: a.uuid.clone() });
        stats.exported += 1;
    }
    items.sort_by(|a, b| a.logical.cmp(&b.logical));

    let manifest = Manifest {
        format: MANIFEST_FORMAT.into(),
        version: 1,
        library_name: lib.path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
        albums: albums
            .iter()
            .map(|al| {
                let mut seen = BTreeSet::new();
                ManifestAlbum {
                    title: al.title.clone(),
                    uuid: al.uuid.clone(),
                    assets: al.assets.iter().filter_map(|pk| by_pk.get(pk)).filter(|p| seen.insert((*p).clone())).cloned().collect(),
                }
            })
            .collect(),
        assets: manifest_assets.into_values().collect(),
    };
    let manifest = serde_json::to_vec_pretty(&manifest).unwrap_or_default();
    PhotosExport { root, items, manifest, stats, warnings }
}

/// Summary of a library for the job editor (album picker).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryInfo {
    pub path: String,
    pub assets: u64,
    pub hidden: u64,
    pub trashed: u64,
    /// User album titles, in library order.
    pub albums: Vec<String>,
    pub warnings: Vec<String>,
}

/// Read a library and summarise it.
pub fn library_info(library: &Path, temp_root: &Path) -> Result<LibraryInfo, AppError> {
    let lib = PhotosLibrary::open(library, temp_root)?;
    Ok(LibraryInfo {
        path: library.to_string_lossy().into_owned(),
        assets: lib.assets.len() as u64,
        hidden: lib.assets.iter().filter(|a| a.hidden).count() as u64,
        trashed: lib.assets.iter().filter(|a| a.trashed).count() as u64,
        albums: lib.albums.iter().map(|a| a.title.clone()).collect(),
        warnings: lib.warnings,
    })
}

/// Open and plan in one step.
pub fn export(library: &Path, opts: &PhotosOptions, temp_root: &Path) -> Result<PhotosExport, AppError> {
    Ok(plan_export(&PhotosLibrary::open(library, temp_root)?, opts))
}

/// Builds synthetic `.photoslibrary` bundles mimicking the real schema.
#[cfg(test)]
pub(crate) mod fixture {
    use rusqlite::Connection;
    use std::path::{Path, PathBuf};

    /// Core Data seconds for a UTC date.
    pub fn cd(y: i32, m: u32, d: u32) -> f64 {
        use chrono::TimeZone;
        (chrono::Utc.with_ymd_and_hms(y, m, d, 12, 0, 0).unwrap().timestamp() - super::CORE_DATA_EPOCH) as f64
    }

    pub struct Asset<'a> {
        pub pk: i64,
        pub uuid: &'a str,
        pub dir: &'a str,
        pub file: &'a str,
        pub original: Option<&'a str>,
        pub created: Option<f64>,
        pub trashed: bool,
        pub hidden: bool,
        pub favorite: bool,
        pub kind: i64,
        /// Write the original file (under `originals/` or `Masters/`).
        pub content: Option<&'a [u8]>,
    }

    impl Default for Asset<'_> {
        fn default() -> Self {
            Self { pk: 0, uuid: "", dir: "0", file: "", original: None, created: None, trashed: false, hidden: false, favorite: false, kind: 0, content: None }
        }
    }

    /// Create a library. `legacy` = `Masters/` layout, `ZGENERICASSET`
    /// table and a `Z_26ASSETS` join table; modern = `originals/`, `ZASSET`,
    /// `Z_28ASSETS`. Albums: (pk, title, kind, trashed, asset pks).
    pub fn library(dir: &Path, name: &str, legacy: bool, assets: &[Asset], albums: &[(i64, &str, i64, bool, &[i64])]) -> PathBuf {
        let lib = dir.join(format!("{name}.photoslibrary"));
        std::fs::create_dir_all(lib.join("database")).unwrap();
        let conn = Connection::open(lib.join("database/Photos.sqlite")).unwrap();
        conn.execute_batch("PRAGMA journal_mode = WAL;").unwrap();
        let (asset_table, join, jalb, jass) = if legacy { ("ZGENERICASSET", "Z_26ASSETS", "Z_26ALBUMS", "Z_34ASSETS") } else { ("ZASSET", "Z_28ASSETS", "Z_28ALBUMS", "Z_3ASSETS") };
        conn.execute_batch(&format!(
            "CREATE TABLE {asset_table} (Z_PK INTEGER PRIMARY KEY, Z_ENT INTEGER, Z_OPT INTEGER, ZDIRECTORY VARCHAR, ZFILENAME VARCHAR, ZUUID VARCHAR,
               ZDATECREATED TIMESTAMP, ZTRASHEDSTATE INTEGER, ZHIDDEN INTEGER, ZKIND INTEGER, ZUNIFORMTYPEIDENTIFIER VARCHAR, ZFAVORITE INTEGER);
             CREATE TABLE ZADDITIONALASSETATTRIBUTES (Z_PK INTEGER PRIMARY KEY, ZASSET INTEGER, ZORIGINALFILENAME VARCHAR, ZTIMEZONEOFFSET INTEGER);
             CREATE TABLE ZGENERICALBUM (Z_PK INTEGER PRIMARY KEY, ZKIND INTEGER, ZTITLE VARCHAR, ZUUID VARCHAR, ZTRASHEDSTATE INTEGER);
             CREATE TABLE {join} ({jalb} INTEGER, {jass} INTEGER, Z_FOK_{jass} INTEGER, PRIMARY KEY ({jalb}, {jass}));
             CREATE TABLE Z_1KEYWORDS (Z_1ASSETATTRIBUTES INTEGER, Z_40KEYWORDS INTEGER);"
        ))
        .unwrap();
        let base = if legacy { "Masters" } else { "originals" };
        for a in assets {
            let uti = if a.kind == 1 { "com.apple.quicktime-movie" } else { "public.jpeg" };
            conn.execute(
                &format!("INSERT INTO {asset_table} (Z_PK, ZDIRECTORY, ZFILENAME, ZUUID, ZDATECREATED, ZTRASHEDSTATE, ZHIDDEN, ZKIND, ZUNIFORMTYPEIDENTIFIER, ZFAVORITE) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)"),
                rusqlite::params![a.pk, a.dir, a.file, a.uuid, a.created, a.trashed as i64, a.hidden as i64, a.kind, uti, a.favorite as i64],
            )
            .unwrap();
            conn.execute("INSERT INTO ZADDITIONALASSETATTRIBUTES (ZASSET, ZORIGINALFILENAME, ZTIMEZONEOFFSET) VALUES (?1, ?2, 0)", rusqlite::params![a.pk, a.original]).unwrap();
            if let Some(c) = a.content {
                let p = lib.join(base).join(a.dir).join(a.file);
                std::fs::create_dir_all(p.parent().unwrap()).unwrap();
                std::fs::write(p, c).unwrap();
            }
        }
        for (pk, title, kind, trashed, members) in albums {
            conn.execute("INSERT INTO ZGENERICALBUM (Z_PK, ZKIND, ZTITLE, ZUUID, ZTRASHEDSTATE) VALUES (?1, ?2, ?3, ?4, ?5)", rusqlite::params![pk, kind, title, format!("ALBUM-{pk}"), *trashed as i64])
                .unwrap();
            for (i, m) in members.iter().enumerate() {
                conn.execute(&format!("INSERT INTO {join} ({jalb}, {jass}, Z_FOK_{jass}) VALUES (?1, ?2, ?3)"), rusqlite::params![pk, m, (i as i64 + 1) * 1024]).unwrap();
            }
        }
        // Keep the WAL (and its rows) next to the database, as a running Photos would.
        std::mem::forget(conn);
        lib
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::{cd, library, Asset};
    use super::*;

    #[test]
    fn parses_source_uris() {
        assert_eq!(parse_source("photos:///Users/me/Pictures/Photos Library.photoslibrary"), Some(PathBuf::from("/Users/me/Pictures/Photos Library.photoslibrary")));
        assert_eq!(parse_source("photos:///Users/me/Pictures/My%20Lib.photoslibrary/"), Some(PathBuf::from("/Users/me/Pictures/My Lib.photoslibrary")));
        assert_eq!(parse_source("photos:///C:/Pics/L.photoslibrary"), Some(PathBuf::from("C:/Pics/L.photoslibrary")));
        assert_eq!(parse_source("photos://C:\\Pics\\L.photoslibrary"), Some(PathBuf::from("C:\\Pics\\L.photoslibrary")));
        assert_eq!(parse_source("photos://"), None);
        assert_eq!(parse_source("/Users/me/Pictures"), None);
        assert_eq!(parse_source("photos:///a%2"), Some(PathBuf::from("/a%2")));
        assert_eq!(source_uri(Path::new("/Users/me/L.photoslibrary")), "photos:///Users/me/L.photoslibrary");
        assert_eq!(export_root(Path::new("/Users/me/Pictures/Photos Library.photoslibrary")), PathBuf::from("/Users/me/Pictures/Photos Library (Photos export)"));
    }

    #[test]
    fn reads_modern_library_and_plans_export() {
        let t = tempfile::tempdir().unwrap();
        let assets = [
            Asset { pk: 1, uuid: "AAAA1111-0000", dir: "A", file: "AAAA1111-0000.jpeg", original: Some("IMG_0001.JPG"), created: Some(cd(2023, 3, 14)), favorite: true, content: Some(b"one"), ..Default::default() },
            Asset { pk: 2, uuid: "BBBB2222-0000", dir: "B", file: "BBBB2222-0000.jpeg", original: Some("IMG_0001.JPG"), created: Some(cd(2023, 3, 20)), content: Some(b"two"), ..Default::default() },
            Asset { pk: 3, uuid: "CCCC3333-0000", dir: "C", file: "CCCC3333-0000.mov", original: Some("clip"), created: Some(cd(2024, 12, 31)), kind: 1, content: Some(b"video"), ..Default::default() },
            Asset { pk: 4, uuid: "DDDD4444-0000", dir: "D", file: "DDDD4444-0000.jpeg", original: Some("trash.jpg"), created: Some(cd(2024, 1, 1)), trashed: true, content: Some(b"t"), ..Default::default() },
            Asset { pk: 5, uuid: "EEEE5555-0000", dir: "E", file: "EEEE5555-0000.jpeg", original: Some("hidden.jpg"), created: Some(cd(2024, 1, 2)), hidden: true, content: Some(b"h"), ..Default::default() },
            Asset { pk: 6, uuid: "FFFF6666-0000", dir: "F", file: "FFFF6666-0000.jpeg", original: Some("cloud.jpg"), created: Some(cd(2024, 1, 3)), content: None, ..Default::default() },
            Asset { pk: 7, uuid: "GGGG7777-0000", dir: "G", file: "GGGG7777-0000.jpeg", original: None, created: None, content: Some(b"undated"), ..Default::default() },
        ];
        let lib = library(t.path(), "Photos Library", false, &assets, &[(10, "Trip", 2, false, &[2, 1, 6]), (11, "Family", 2, false, &[3, 5]), (12, "Smart", 4, false, &[1]), (13, "Old", 2, true, &[1])]);
        let l = PhotosLibrary::open(&lib, t.path()).unwrap();
        assert_eq!(l.assets.len(), 7);
        assert!(l.warnings.is_empty(), "{:?}", l.warnings);
        assert_eq!(l.albums.iter().map(|a| a.title.as_str()).collect::<Vec<_>>(), ["Trip", "Family"]);
        assert_eq!(l.albums[0].assets, [2, 1, 6], "album order follows Z_FOK");
        // The temporary copy is gone.
        assert!(!std::fs::read_dir(t.path()).unwrap().any(|e| e.unwrap().file_name().to_string_lossy().starts_with("ufop-photos-")));

        let e = plan_export(&l, &PhotosOptions::default());
        let root = t.path().join("Photos Library (Photos export)");
        assert_eq!(e.root, root);
        let rel: Vec<String> = e.items.iter().map(|i| rel_string(i.logical.strip_prefix(&root).unwrap())).collect();
        assert_eq!(rel, ["Photos/2023/03/IMG_0001 (BBBB2222).JPG", "Photos/2023/03/IMG_0001.JPG", "Photos/2024/12/clip.mov", "Photos/Undated/GGGG7777-0000.jpeg"]);
        assert_eq!(e.stats, ExportStats { exported: 4, skipped_hidden: 1, skipped_trashed: 1, skipped_album_filter: 0, missing_originals: 1 });
        assert_eq!(e.warnings.len(), 1);
        assert!(e.warnings[0].contains("cloud.jpg") && e.warnings[0].contains("FFFF6666-0000"), "{}", e.warnings[0]);

        let m: Manifest = serde_json::from_slice(&e.manifest).unwrap();
        assert_eq!(m.format, MANIFEST_FORMAT);
        assert_eq!(m.library_name, "Photos Library");
        assert_eq!(m.albums.len(), 2);
        assert_eq!(m.albums[0].title, "Trip");
        assert_eq!(m.albums[0].uuid.as_deref(), Some("ALBUM-10"));
        assert_eq!(m.albums[0].assets, ["Photos/2023/03/IMG_0001 (BBBB2222).JPG", "Photos/2023/03/IMG_0001.JPG"]);
        assert_eq!(m.albums[1].assets, ["Photos/2024/12/clip.mov"], "hidden member is not exported");
        let first = m.assets.iter().find(|a| a.uuid == "AAAA1111-0000").unwrap();
        assert!(first.favorite);
        assert_eq!(first.albums, ["Trip"], "smart and deleted albums are not listed");
        assert_eq!(first.created.as_deref(), Some("2023-03-14T12:00:00+00:00"));
        assert_eq!(first.kind, "photo");
        assert_eq!(m.assets.iter().find(|a| a.uuid == "CCCC3333-0000").unwrap().kind, "video");
        // Deterministic.
        assert_eq!(plan_export(&l, &PhotosOptions::default()).manifest, e.manifest);
    }

    #[test]
    fn options_include_hidden_trashed_and_filter_albums() {
        let t = tempfile::tempdir().unwrap();
        let assets = [
            Asset { pk: 1, uuid: "U1", dir: "A", file: "U1.jpeg", original: Some("a.jpg"), created: Some(cd(2022, 5, 1)), content: Some(b"a"), ..Default::default() },
            Asset { pk: 2, uuid: "U2", dir: "A", file: "U2.jpeg", original: Some("b.jpg"), created: Some(cd(2022, 5, 2)), hidden: true, content: Some(b"b"), ..Default::default() },
            Asset { pk: 3, uuid: "U3", dir: "A", file: "U3.jpeg", original: Some("c.jpg"), created: Some(cd(2022, 5, 3)), trashed: true, content: Some(b"c"), ..Default::default() },
        ];
        let lib = library(t.path(), "L", false, &assets, &[(1, "Keep", 2, false, &[2, 3]), (2, "Other", 2, false, &[1])]);
        let l = PhotosLibrary::open(&lib, t.path()).unwrap();
        let all = plan_export(&l, &PhotosOptions { include_hidden: true, include_trashed: true, albums: vec![] });
        assert_eq!(all.items.len(), 3);
        let m: Manifest = serde_json::from_slice(&all.manifest).unwrap();
        assert!(m.assets.iter().any(|a| a.hidden) && m.assets.iter().any(|a| a.trashed));
        let only = plan_export(&l, &PhotosOptions { include_hidden: true, include_trashed: false, albums: vec![" keep ".into(), "Missing".into()] });
        assert_eq!(only.items.iter().map(|i| i.uuid.as_str()).collect::<Vec<_>>(), ["U2"]);
        assert_eq!(only.stats.skipped_album_filter, 1);
        assert_eq!(only.stats.skipped_trashed, 1);
        assert!(only.warnings.iter().any(|w| w.contains("'missing'")));
        let m: Manifest = serde_json::from_slice(&only.manifest).unwrap();
        assert_eq!(m.albums.len(), 1);
        assert_eq!(m.albums[0].title, "Keep");
    }

    #[test]
    fn legacy_masters_layout_and_join_table() {
        let t = tempfile::tempdir().unwrap();
        let assets = [Asset { pk: 1, uuid: "L1", dir: "2016/07/04/20160704-101010", file: "IMG_0100.JPG", original: Some("IMG_0100.JPG"), created: Some(cd(2016, 7, 4)), content: Some(b"old"), ..Default::default() }];
        let lib = library(t.path(), "Old", true, &assets, &[(5, "Summer", 2, false, &[1])]);
        let l = PhotosLibrary::open(&lib, t.path()).unwrap();
        assert_eq!(l.albums[0].assets, [1]);
        let e = plan_export(&l, &PhotosOptions::default());
        assert_eq!(e.items.len(), 1);
        assert_eq!(e.items[0].original, lib.join("Masters/2016/07/04/20160704-101010/IMG_0100.JPG"));
        assert!(e.items[0].logical.ends_with("Photos/2016/07/IMG_0100.JPG"));
    }

    #[test]
    fn sparse_schema_degrades_to_warnings() {
        let t = tempfile::tempdir().unwrap();
        let lib = t.path().join("Bare.photoslibrary");
        std::fs::create_dir_all(lib.join("database")).unwrap();
        std::fs::create_dir_all(lib.join("originals/0")).unwrap();
        std::fs::write(lib.join("originals/0/x.heic"), b"x").unwrap();
        let c = Connection::open(lib.join("database/Photos.sqlite")).unwrap();
        c.execute_batch("CREATE TABLE ZASSET (Z_PK INTEGER PRIMARY KEY, ZDIRECTORY TEXT, ZFILENAME TEXT); INSERT INTO ZASSET VALUES (1, '0', 'x.heic'), (2, '../..', 'evil');").unwrap();
        drop(c);
        let l = PhotosLibrary::open(&lib, t.path()).unwrap();
        assert!(l.warnings.iter().any(|w| w.contains("ZUUID")));
        assert!(l.warnings.iter().any(|w| w.contains("album table")));
        let e = plan_export(&l, &PhotosOptions::default());
        assert_eq!(e.items.len(), 1, "path traversal in ZDIRECTORY is refused");
        assert!(e.items[0].logical.ends_with("Photos/Undated/x.heic"));
        assert_eq!(e.stats.missing_originals, 1);
    }

    #[test]
    fn not_a_library_is_an_error() {
        let t = tempfile::tempdir().unwrap();
        assert!(PhotosLibrary::open(&t.path().join("none.photoslibrary"), t.path()).is_err());
        let lib = t.path().join("Empty.photoslibrary");
        std::fs::create_dir_all(lib.join("database")).unwrap();
        assert!(PhotosLibrary::open(&lib, t.path()).unwrap_err().user_message().contains("Photos.sqlite"));
        Connection::open(lib.join("database/Photos.sqlite")).unwrap().execute_batch("CREATE TABLE T (a);").unwrap();
        assert!(PhotosLibrary::open(&lib, t.path()).is_err());
    }

    #[test]
    fn summarises_a_library() {
        let t = tempfile::tempdir().unwrap();
        let assets = [
            Asset { pk: 1, uuid: "1", file: "1.jpg", hidden: true, ..Default::default() },
            Asset { pk: 2, uuid: "2", file: "2.jpg", trashed: true, ..Default::default() },
            Asset { pk: 3, uuid: "3", file: "3.jpg", ..Default::default() },
        ];
        let lib = library(t.path(), "L", false, &assets, &[(1, "B album", 2, false, &[3]), (2, "A album", 2, false, &[]), (3, "Gone", 2, true, &[])]);
        let i = library_info(&lib, t.path()).unwrap();
        assert_eq!((i.assets, i.hidden, i.trashed), (3, 1, 1));
        assert_eq!(i.albums, ["B album", "A album"]);
    }

    #[test]
    fn finds_libraries_in_a_folder() {
        let t = tempfile::tempdir().unwrap();
        library(t.path(), "Photos Library", false, &[], &[]);
        std::fs::create_dir_all(t.path().join("Another.photoslibrary")).unwrap();
        std::fs::create_dir_all(t.path().join("NotALibrary")).unwrap();
        let c = find_libraries(t.path());
        assert_eq!(c.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["Photos Library", "Another"]);
        assert!(c[0].is_default && c[0].has_database);
        assert!(!c[1].has_database);
        assert!(c[0].source.starts_with(SOURCE_SCHEME));
        assert_eq!(parse_source(&c[0].source).unwrap(), PathBuf::from(&c[0].path));
    }
}
