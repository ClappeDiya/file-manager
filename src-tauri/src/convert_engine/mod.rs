//! File conversion: images (resize and change format, with the `image`
//! crate) and office documents (through LibreOffice in headless mode).
//!
//! Used by the automation actions `convert_image` / `convert_document` and
//! by the file manager's Convert dialog. Conversions write next to a
//! temporary name and rename into place, so a failed conversion never
//! leaves a half-written file.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Image formats that can be written.
pub const IMAGE_FORMATS: &[&str] = &["jpeg", "png", "webp", "gif", "bmp", "tiff"];
/// Document formats LibreOffice is asked to produce.
pub const DOCUMENT_FORMATS: &[&str] = &["pdf", "docx", "odt", "rtf", "txt", "html", "xlsx", "ods", "csv", "pptx", "odp"];
pub const DEFAULT_QUALITY: u8 = 85;
const MAX_SIDE: u32 = 20_000;

/// How to convert an image.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ImageSpec {
    /// Target format (one of [`IMAGE_FORMATS`]); `None` keeps the source format.
    #[serde(default)]
    pub format: Option<String>,
    /// Fit inside this box, keeping the aspect ratio; never enlarges.
    #[serde(default)]
    pub max_width: Option<u32>,
    #[serde(default)]
    pub max_height: Option<u32>,
    /// JPEG quality 1–100.
    #[serde(default)]
    pub quality: Option<u8>,
}

/// Decode an image with size limits (also used for thumbnails).
pub fn decode(path: &Path) -> Result<image::DynamicImage, String> {
    let mut reader = image::ImageReader::open(path)
        .map_err(|e| format!("Cannot open {}: {e}", path.display()))?
        .with_guessed_format()
        .map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SIDE);
    limits.max_image_height = Some(MAX_SIDE);
    limits.max_alloc = Some(512 * 1024 * 1024);
    reader.limits(limits);
    reader.decode().map_err(|e| format!("{} is not a readable image: {e}", path.display()))
}

fn format_of(name: &str) -> Result<image::ImageFormat, String> {
    Ok(match name.to_ascii_lowercase().as_str() {
        "jpeg" | "jpg" => image::ImageFormat::Jpeg,
        "png" => image::ImageFormat::Png,
        "webp" => image::ImageFormat::WebP,
        "gif" => image::ImageFormat::Gif,
        "bmp" => image::ImageFormat::Bmp,
        "tiff" | "tif" => image::ImageFormat::Tiff,
        other => return Err(format!("Unsupported image format '{other}' (use {})", IMAGE_FORMATS.join(", "))),
    })
}

fn extension(format: image::ImageFormat) -> &'static str {
    match format {
        image::ImageFormat::Jpeg => "jpg",
        image::ImageFormat::Tiff => "tif",
        f => f.extensions_str().first().copied().unwrap_or("img"),
    }
}

impl ImageSpec {
    pub fn validate(&self) -> Result<(), String> {
        if let Some(f) = &self.format {
            format_of(f)?;
        }
        for (what, v) in [("width", self.max_width), ("height", self.max_height)] {
            if v.is_some_and(|v| v == 0 || v > MAX_SIDE) {
                return Err(format!("The largest {what} is 1–{MAX_SIDE} pixels"));
            }
        }
        if self.quality.is_some_and(|q| q == 0 || q > 100) {
            return Err("Quality is 1–100".into());
        }
        if self.format.is_none() && self.max_width.is_none() && self.max_height.is_none() {
            return Err("Choose a format or a largest size".into());
        }
        Ok(())
    }

    /// File name of the converted `src`: same stem, the target format's extension.
    pub fn output_name(&self, src: &Path) -> Result<String, String> {
        let stem = src.file_stem().map(|s| s.to_string_lossy().into_owned()).ok_or("Cannot determine filename")?;
        let ext = match &self.format {
            Some(f) => extension(format_of(f)?).to_string(),
            None => src.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_else(|| "png".into()),
        };
        Ok(format!("{stem}.{ext}"))
    }
}

/// Convert `src` to `dest` (which may be `src`). Returns the new size.
pub fn convert_image(src: &Path, dest: &Path, spec: &ImageSpec) -> Result<(u32, u32), String> {
    spec.validate()?;
    let mut img = decode(src)?;
    let (w, h) = (img.width(), img.height());
    let (bw, bh) = (spec.max_width.unwrap_or(u32::MAX), spec.max_height.unwrap_or(u32::MAX));
    if w > bw || h > bh {
        img = img.resize(bw.min(w), bh.min(h), image::imageops::FilterType::Lanczos3);
    }
    let format = match &spec.format {
        Some(f) => format_of(f)?,
        None => image::ImageFormat::from_path(src).map_err(|_| format!("Cannot tell the format of {}; choose one", src.display()))?,
    };
    let mut bytes = Vec::new();
    let mut out = std::io::Cursor::new(&mut bytes);
    match format {
        image::ImageFormat::Jpeg => {
            // JPEG has no alpha channel.
            let rgb = image::DynamicImage::ImageRgb8(img.to_rgb8());
            let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, spec.quality.unwrap_or(DEFAULT_QUALITY));
            rgb.write_with_encoder(enc).map_err(|e| format!("Cannot write JPEG: {e}"))?;
        }
        f => img.write_to(&mut out, f).map_err(|e| format!("Cannot write {}: {e}", extension(f)))?,
    }
    write_atomic(dest, &bytes)?;
    Ok((img.width(), img.height()))
}

fn write_atomic(dest: &Path, bytes: &[u8]) -> Result<(), String> {
    let name = dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = dest.with_file_name(crate::connectors::staging_name(&name, &format!(".{}.ufop-tmp", uuid::Uuid::new_v4().simple())));
    std::fs::write(&tmp, bytes).and_then(|_| std::fs::rename(&tmp, dest)).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("Cannot write {}: {e}", dest.display())
    })
}

// ── Documents ──

/// The LibreOffice program: `$UFOP_SOFFICE`, `soffice`/`libreoffice` on
/// PATH, or the standard install locations.
pub fn find_office() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("UFOP_SOFFICE").map(PathBuf::from) {
        return p.is_file().then_some(p);
    }
    let names: &[&str] = if cfg!(windows) { &["soffice.exe"] } else { &["soffice", "libreoffice"] };
    let on_path = std::env::var_os("PATH").into_iter().flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>()).flat_map(|d| names.iter().map(move |n| d.join(n)));
    let known: Vec<PathBuf> = if cfg!(target_os = "macos") {
        vec!["/Applications/LibreOffice.app/Contents/MacOS/soffice".into()]
    } else if cfg!(windows) {
        ["ProgramFiles", "ProgramFiles(x86)"].iter().filter_map(std::env::var_os).map(|p| PathBuf::from(p).join("LibreOffice").join("program").join("soffice.exe")).collect()
    } else {
        vec!["/usr/lib/libreoffice/program/soffice".into(), "/opt/libreoffice/program/soffice".into(), "/snap/bin/libreoffice".into()]
    };
    on_path.chain(known).find(|p| p.is_file())
}

pub fn validate_document_format(to: &str) -> Result<String, String> {
    let to = to.trim().trim_start_matches('.').to_ascii_lowercase();
    if DOCUMENT_FORMATS.contains(&to.as_str()) {
        Ok(to)
    } else {
        Err(format!("Unsupported document format '{to}' (use {})", DOCUMENT_FORMATS.join(", ")))
    }
}

/// Convert `src` to format `to` with LibreOffice; the result is written to
/// `dest` (a file path). A private LibreOffice profile is used, so a running
/// LibreOffice doesn't interfere.
pub async fn convert_document(src: &Path, dest: &Path, to: &str, timeout: Duration) -> Result<(), String> {
    let to = validate_document_format(to)?;
    let office = find_office().ok_or("LibreOffice was not found. Install it, or set UFOP_SOFFICE to its soffice program.")?;
    let work = WorkDir::new()?;
    let (profile, out) = (work.0.join("profile"), work.0.join("out"));
    std::fs::create_dir_all(&out).map_err(|e| format!("Cannot create a working folder: {e}"))?;
    let profile_url = url::Url::from_directory_path(&profile).map_err(|_| "Cannot build the LibreOffice profile path".to_string())?;
    // `txt` needs an explicit filter to keep UTF-8.
    let target = if to == "txt" { "txt:Text (encoded):UTF8".to_string() } else { to.clone() };
    let mut cmd = tokio::process::Command::new(&office);
    cmd.arg(format!("-env:UserInstallation={profile_url}"))
        .args(["--headless", "--norestore", "--nologo", "--nodefault", "--nolockcheck", "--convert-to", &target, "--outdir"])
        .arg(&out)
        .arg(src)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let output = tokio::time::timeout(timeout, cmd.output())
        .await
        .map_err(|_| format!("LibreOffice did not finish within {} seconds", timeout.as_secs()))?
        .map_err(|e| format!("Could not start LibreOffice: {e}"))?;
    let stem = src.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let produced = out.join(format!("{stem}.{to}"));
    if !produced.is_file() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().lines().last().unwrap_or_default().to_string();
        return Err(format!("LibreOffice could not convert {} to {to}{}", src.display(), if detail.is_empty() { String::new() } else { format!(": {detail}") }));
    }
    let bytes = std::fs::read(&produced).map_err(|e| format!("Cannot read the converted file: {e}"))?;
    write_atomic(dest, &bytes)
}

/// A private working folder, removed when dropped.
struct WorkDir(PathBuf);

impl WorkDir {
    fn new() -> Result<Self, String> {
        let dir = std::env::temp_dir().join(format!("ufop-convert-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir(&dir).map_err(|e| format!("Cannot create a working folder: {e}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
        }
        Ok(Self(dir))
    }
}

impl Drop for WorkDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// File name of `src` converted to document format `to`.
pub fn document_output_name(src: &Path, to: &str) -> Result<String, String> {
    let to = validate_document_format(to)?;
    let stem = src.file_stem().map(|s| s.to_string_lossy().into_owned()).ok_or("Cannot determine filename")?;
    Ok(format!("{stem}.{to}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn photo(dir: &Path, name: &str, w: u32, h: u32) -> PathBuf {
        let p = dir.join(name);
        image::RgbaImage::from_fn(w, h, |x, y| image::Rgba([(x % 256) as u8, (y % 256) as u8, 90, if x < 4 { 0 } else { 255 }])).save(&p).unwrap();
        p
    }

    // Review (WS-Z2): the temporary name went past 255 bytes for a long name.
    #[test]
    fn a_long_name_is_written() {
        let d = tempfile::tempdir().unwrap();
        let dest = d.path().join(format!("{}.png", "p".repeat(240)));
        write_atomic(&dest, b"x").unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"x");
    }

    #[test]
    fn resizes_within_the_box_and_converts_formats() {
        let d = tempfile::tempdir().unwrap();
        let src = photo(d.path(), "wide.png", 400, 200);
        let spec = ImageSpec { format: Some("jpeg".into()), max_width: Some(100), max_height: Some(100), quality: Some(70) };
        assert_eq!(spec.output_name(&src).unwrap(), "wide.jpg");
        let dest = d.path().join("wide.jpg");
        assert_eq!(convert_image(&src, &dest, &spec).unwrap(), (100, 50), "aspect ratio kept");
        let back = image::open(&dest).unwrap();
        assert_eq!((back.width(), back.height()), (100, 50));
        assert_eq!(image::ImageFormat::from_path(&dest).unwrap(), image::ImageFormat::Jpeg);

        // Never enlarged; same format in place.
        let small = photo(d.path(), "small.png", 40, 30);
        let keep = ImageSpec { max_width: Some(1000), ..Default::default() };
        assert_eq!(keep.output_name(&small).unwrap(), "small.png");
        assert_eq!(convert_image(&small, &small, &keep).unwrap(), (40, 30));
        assert!(image::open(&small).unwrap().color().has_alpha(), "PNG keeps transparency");

        for f in IMAGE_FORMATS {
            let spec = ImageSpec { format: Some((*f).into()), ..Default::default() };
            let out = d.path().join(spec.output_name(&src).unwrap());
            convert_image(&src, &out, &spec).unwrap();
            assert_eq!(image::open(&out).unwrap().width(), 400, "{f}");
        }
        // No temporary files are left behind.
        assert!(std::fs::read_dir(d.path()).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().contains("ufop-tmp")));
    }

    #[test]
    fn rejects_bad_specs_and_non_images() {
        let d = tempfile::tempdir().unwrap();
        for bad in [
            ImageSpec::default(),
            ImageSpec { format: Some("heic".into()), ..Default::default() },
            ImageSpec { max_width: Some(0), ..Default::default() },
            ImageSpec { format: Some("png".into()), quality: Some(101), ..Default::default() },
        ] {
            assert!(bad.validate().is_err(), "{bad:?}");
        }
        let text = d.path().join("notes.png");
        std::fs::write(&text, "not really a png").unwrap();
        let err = convert_image(&text, &d.path().join("x.png"), &ImageSpec { max_width: Some(10), ..Default::default() }).unwrap_err();
        assert!(err.contains("not a readable image"), "{err}");
        assert!(!d.path().join("x.png").exists());
    }

    #[test]
    fn document_formats_and_names() {
        assert_eq!(validate_document_format(".PDF").unwrap(), "pdf");
        assert!(validate_document_format("exe").is_err());
        assert_eq!(document_output_name(Path::new("/d/Report.docx"), "pdf").unwrap(), "Report.pdf");
    }

    /// Real LibreOffice, when installed.
    #[tokio::test]
    async fn converts_documents_with_libreoffice() {
        if find_office().is_none() {
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("memo.txt");
        std::fs::write(&src, "Quarterly memo about kestrels").unwrap();
        let pdf = d.path().join("memo.pdf");
        convert_document(&src, &pdf, "pdf", Duration::from_secs(180)).await.unwrap();
        assert!(std::fs::read(&pdf).unwrap().starts_with(b"%PDF-"));
        // And the text survives the round trip (read with the search index's PDF reader).
        let text = ufop_core::doctext::extract_text(&pdf, 10_000).unwrap().unwrap();
        assert!(text.contains("kestrels"), "{text}");
        let odt = d.path().join("memo.odt");
        convert_document(&src, &odt, "odt", Duration::from_secs(180)).await.unwrap();
        assert!(ufop_core::doctext::extract_text(&odt, 10_000).unwrap().unwrap().contains("kestrels"));
        let missing = convert_document(&d.path().join("none.txt"), &d.path().join("none.pdf"), "pdf", Duration::from_secs(180)).await.unwrap_err();
        assert!(missing.contains("could not convert"), "{missing}");
    }
}
