//! Third-party bookmark / connection profile import (FileZilla, Cyberduck, WinSCP, Transmit).
//!
//! Each parser reads the native format of a popular file-transfer client and
//! produces a list of [`ImportedProfile`] structs that can be converted into
//! [`ConnectionProfile`] entries and saved through the [`ConnectionManager`].
//!
//! XML is parsed with simple string helpers (no external XML crate required).

use crate::core::error::AppError;
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use std::path::Path;

// ──────────────────────────────────────────────
// Public types
// ──────────────────────────────────────────────

/// A connection profile imported from a third-party application.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportedProfile {
    pub name: String,
    /// Normalised protocol string: "sftp", "ftp", "ftps", "webdav", "s3".
    pub protocol: String,
    pub host: String,
    pub port: Option<u16>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub remote_path: Option<String>,
    /// Source application identifier: "filezilla", "cyberduck", "winscp", "transmit".
    pub source_app: String,
}

/// Summary returned after a third-party import completes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThirdPartyImportResult {
    pub imported_connections: u32,
    pub skipped: u32,
    pub errors: Vec<String>,
    pub source_app: String,
}

// ──────────────────────────────────────────────
// Tiny XML helpers (no external crate)
// ──────────────────────────────────────────────

/// Extract the text content between `<tag>` and `</tag>`.
/// Returns `None` when the tag is missing or empty.
fn xml_tag_value<'a>(xml: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    let val = xml[start..end].trim();
    if val.is_empty() {
        None
    } else {
        Some(val)
    }
}

/// Split `xml` into substrings each enclosed by `<tag>...</tag>` (non-nested).
fn xml_split_blocks<'a>(xml: &'a str, tag: &str) -> Vec<&'a str> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let mut results = Vec::new();
    let mut haystack = xml;

    while let Some(start) = haystack.find(&open) {
        if let Some(end_offset) = haystack[start..].find(&close) {
            let end = start + end_offset + close.len();
            results.push(&haystack[start..end]);
            haystack = &haystack[end..];
        } else {
            break;
        }
    }
    results
}

/// Decode XML character entities in a string (&amp;, &lt;, &gt;, &quot;, &apos;).
fn xml_unescape(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

// ──────────────────────────────────────────────
// Plist helpers (Apple XML plist used by Cyberduck / Transmit)
// ──────────────────────────────────────────────

/// From a plist `<dict>` block, extract the `<string>` value for a given `<key>`.
fn plist_string_value<'a>(dict_xml: &'a str, key: &str) -> Option<&'a str> {
    let key_tag = format!("<key>{key}</key>");
    let pos = dict_xml.find(&key_tag)?;
    let after_key = &dict_xml[pos + key_tag.len()..];
    // Skip whitespace
    let after_key = after_key.trim_start();
    // The next element should be <string>...</string>
    let open = "<string>";
    let close = "</string>";
    if !after_key.starts_with(open) {
        return None;
    }
    let start = open.len();
    let end = after_key.find(close)?;
    let val = after_key[start..end].trim();
    if val.is_empty() {
        None
    } else {
        Some(val)
    }
}

/// From a plist `<dict>` block, extract an `<integer>` value for a given `<key>`.
fn plist_integer_value(dict_xml: &str, key: &str) -> Option<i64> {
    let key_tag = format!("<key>{key}</key>");
    let pos = dict_xml.find(&key_tag)?;
    let after_key = &dict_xml[pos + key_tag.len()..];
    let after_key = after_key.trim_start();
    let open = "<integer>";
    let close = "</integer>";
    if !after_key.starts_with(open) {
        return None;
    }
    let start = open.len();
    let end = after_key.find(close)?;
    after_key[start..end].trim().parse().ok()
}

// ──────────────────────────────────────────────
// FileZilla  (sitemanager.xml)
// ──────────────────────────────────────────────

/// Map FileZilla's numeric `<Protocol>` element to a normalised string.
fn filezilla_protocol(code: &str) -> &'static str {
    match code.trim() {
        "0" => "ftp",
        "1" => "sftp",
        "3" | "4" => "ftps",
        "6" => "s3",
        _ => "ftp",
    }
}

/// Parse a FileZilla `sitemanager.xml` file.
///
/// The XML looks like:
/// ```xml
/// <FileZilla3>
///   <Servers>
///     <Server>
///       <Name>My Server</Name>
///       <Host>example.com</Host>
///       <Port>22</Port>
///       <Protocol>1</Protocol>
///       <User>admin</User>
///       <Pass encoding="base64">cGFzcw==</Pass>
///       <RemoteDir>1 0 5 home</RemoteDir>
///     </Server>
///     <Folder expanded="1">
///       <Server>...</Server>
///     </Folder>
///   </Servers>
/// </FileZilla3>
/// ```
pub fn parse_filezilla(path: &Path) -> Result<Vec<ImportedProfile>, AppError> {
    let content = std::fs::read_to_string(path)?;

    // Quick sanity check
    if !content.contains("<FileZilla3") && !content.contains("<Server>") {
        return Err(AppError::connection(
            "Not a valid FileZilla sitemanager.xml file",
            "Select a FileZilla sitemanager.xml file.",
        ));
    }

    let mut profiles = Vec::new();

    for block in xml_split_blocks(&content, "Server") {
        let name = xml_tag_value(block, "Name")
            .map(xml_unescape)
            .unwrap_or_else(|| "Unnamed".to_string());
        let host = match xml_tag_value(block, "Host") {
            Some(h) => xml_unescape(h),
            None => continue, // skip entries without a host
        };
        let port: Option<u16> = xml_tag_value(block, "Port").and_then(|p| p.parse().ok());
        let protocol = xml_tag_value(block, "Protocol")
            .map(filezilla_protocol)
            .unwrap_or("ftp");
        let username = xml_tag_value(block, "User").map(xml_unescape);

        // FileZilla encodes passwords as base64 when `encoding="base64"` is present.
        let password = extract_filezilla_password(block);

        // FileZilla stores remote dir in a compact encoded format: "1 0 5 home 4 data"
        // We simplify: take tokens that look like path segments.
        let remote_path = xml_tag_value(block, "RemoteDir").map(|rd| {
            parse_filezilla_remote_dir(rd)
        });

        profiles.push(ImportedProfile {
            name,
            protocol: protocol.to_string(),
            host,
            port,
            username,
            password,
            remote_path,
            source_app: "filezilla".to_string(),
        });
    }

    Ok(profiles)
}

/// Extract and decode a FileZilla `<Pass>` element.
/// Handles both `encoding="base64"` and plain-text passwords.
fn extract_filezilla_password(block: &str) -> Option<String> {
    // Find <Pass ...>...</Pass>
    let pass_start = block.find("<Pass")?;
    let pass_close_end = block[pass_start..].find("</Pass>")? + pass_start + "</Pass>".len();
    let pass_element = &block[pass_start..pass_close_end];

    // Get the inner text (after the closing `>` of the opening tag)
    let inner_start = pass_element.find('>')? + 1;
    let inner_end = pass_element.rfind("</Pass>")?;
    let inner = pass_element[inner_start..inner_end].trim();

    if inner.is_empty() {
        return None;
    }

    if pass_element.contains("encoding=\"base64\"") || pass_element.contains("encoding='base64'") {
        // Decode base64
        match base64::engine::general_purpose::STANDARD.decode(inner) {
            Ok(bytes) => String::from_utf8(bytes).ok(),
            Err(_) => Some(inner.to_string()), // Fallback: treat as plain text
        }
    } else {
        Some(xml_unescape(inner))
    }
}

/// Parse FileZilla's encoded remote directory format.
///
/// Format is a sequence of `<length> <segment>` pairs preceded by a type byte.
/// Example: `1 0 5 home 4 data` means `/home/data`.
/// We attempt to reconstruct a readable path; on failure, return "/".
fn parse_filezilla_remote_dir(raw: &str) -> String {
    let raw = raw.trim();
    if raw.is_empty() {
        return "/".to_string();
    }

    // The format starts with a type indicator and count.
    // Simplified parser: skip leading numbers, collect path-like segments.
    let tokens: Vec<&str> = raw.split_whitespace().collect();
    if tokens.len() < 3 {
        return "/".to_string();
    }

    // Skip the first two tokens (type + count), then alternate (length, segment).
    let mut segments = Vec::new();
    let mut i = 2;
    while i + 1 < tokens.len() {
        // tokens[i] should be the length, tokens[i+1] the segment text
        if tokens[i].parse::<usize>().is_ok() {
            segments.push(tokens[i + 1]);
            i += 2;
        } else {
            // Not the expected format; bail out
            break;
        }
    }

    if segments.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", segments.join("/"))
    }
}

// ──────────────────────────────────────────────
// Cyberduck  (.duck files - XML plist)
// ──────────────────────────────────────────────

/// Map Cyberduck protocol identifiers to normalised protocol strings.
fn cyberduck_protocol(proto: &str) -> &'static str {
    match proto {
        "sftp" | "ssh" => "sftp",
        "ftp" => "ftp",
        "ftp-ssl" | "ftps" | "ftp-tls" => "ftps",
        "dav" | "davs" | "webdav" | "webdavs" => "webdav",
        "s3" | "s3-ssl" => "s3",
        "smb" => "smb",
        _ => {
            if proto.contains("sftp") || proto.contains("ssh") {
                "sftp"
            } else if proto.contains("ftp") {
                "ftp"
            } else if proto.contains("dav") {
                "webdav"
            } else if proto.contains("s3") {
                "s3"
            } else {
                "ftp"
            }
        }
    }
}

/// Parse a Cyberduck `.duck` bookmark file (XML plist).
///
/// Structure:
/// ```xml
/// <?xml version="1.0" encoding="UTF-8"?>
/// <!DOCTYPE plist ...>
/// <plist version="1.0">
///   <dict>
///     <key>Protocol</key>
///     <string>sftp</string>
///     <key>Hostname</key>
///     <string>example.com</string>
///     <key>Port</key>
///     <integer>22</integer>
///     <key>Username</key>
///     <string>admin</string>
///     <key>Nickname</key>
///     <string>My Server</string>
///     <key>Path</key>
///     <string>/home/admin</string>
///   </dict>
/// </plist>
/// ```
pub fn parse_cyberduck(path: &Path) -> Result<Vec<ImportedProfile>, AppError> {
    // A .duck file may be a single bookmark or a directory of .duck files.
    // Check if the path is a directory first (before trying to read as file).
    let metadata = std::fs::metadata(path)?;
    if metadata.is_dir() {
        return parse_cyberduck_directory(path);
    }

    let content = std::fs::read_to_string(path)?;

    // Sanity check
    if !content.contains("<plist") && !content.contains("<dict>") {
        return Err(AppError::connection(
            "Not a valid Cyberduck bookmark file",
            "Select a .duck file or a folder containing .duck files.",
        ));
    }

    let profile = parse_single_cyberduck_bookmark(&content)?;
    Ok(vec![profile])
}

/// Parse a directory of `.duck` files.
fn parse_cyberduck_directory(dir: &Path) -> Result<Vec<ImportedProfile>, AppError> {
    let mut profiles = Vec::new();
    let entries = std::fs::read_dir(dir)?;

    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("duck") {
            match std::fs::read_to_string(&path) {
                Ok(content) => {
                    if let Ok(profile) = parse_single_cyberduck_bookmark(&content) {
                        profiles.push(profile);
                    }
                }
                Err(e) => {
                    tracing::warn!("Skipping {}: {e}", path.display());
                }
            }
        }
    }

    Ok(profiles)
}

/// Parse a single Cyberduck plist bookmark XML string.
fn parse_single_cyberduck_bookmark(xml: &str) -> Result<ImportedProfile, AppError> {
    let hostname = plist_string_value(xml, "Hostname")
        .map(xml_unescape)
        .ok_or_else(|| {
            AppError::connection(
                "Cyberduck bookmark is missing Hostname",
                "The bookmark file may be corrupted.",
            )
        })?;

    let protocol_raw = plist_string_value(xml, "Protocol").unwrap_or("ftp");
    let protocol = cyberduck_protocol(protocol_raw);

    let nickname = plist_string_value(xml, "Nickname").map(xml_unescape);
    let name = nickname.unwrap_or_else(|| hostname.clone());

    let port: Option<u16> = plist_integer_value(xml, "Port").and_then(|v| u16::try_from(v).ok());
    let username = plist_string_value(xml, "Username").map(xml_unescape);
    let remote_path = plist_string_value(xml, "Path").map(xml_unescape);

    Ok(ImportedProfile {
        name,
        protocol: protocol.to_string(),
        host: hostname,
        port,
        username,
        password: None, // Cyberduck stores passwords in the OS keychain
        remote_path,
        source_app: "cyberduck".to_string(),
    })
}

// ──────────────────────────────────────────────
// WinSCP  (WinSCP.ini)
// ──────────────────────────────────────────────

/// Map WinSCP's numeric `FSProtocol` to a normalised protocol string.
fn winscp_protocol(fs_protocol: &str) -> &'static str {
    match fs_protocol.trim() {
        "0" => "ftp",   // SCP (treated as sftp for our purposes)
        "5" => "sftp",
        "2" => "ftp",
        "7" => "ftps",
        "4" => "webdav",
        "1" => "s3",
        _ => "sftp", // default to sftp
    }
}

/// Parse a WinSCP `WinSCP.ini` configuration file.
///
/// Format (INI with `[Sessions\<name>]` sections):
/// ```ini
/// [Sessions\My%20Server]
/// HostName=example.com
/// PortNumber=22
/// UserName=admin
/// FSProtocol=5
/// RemoteDirectory=/home/admin
/// Password=...
/// ```
pub fn parse_winscp(path: &Path) -> Result<Vec<ImportedProfile>, AppError> {
    let content = std::fs::read_to_string(path)?;

    // Sanity check
    if !content.contains("[Sessions\\") && !content.contains("[Sessions/") {
        return Err(AppError::connection(
            "Not a valid WinSCP.ini file",
            "Select a WinSCP.ini or WinSCP configuration file.",
        ));
    }

    let mut profiles = Vec::new();
    let lines: Vec<&str> = content.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i].trim();

        // Detect session section headers: [Sessions\Name] or [Sessions/Name]
        if (line.starts_with("[Sessions\\") || line.starts_with("[Sessions/"))
            && line.ends_with(']')
        {
            // Extract session name (URL-decoded)
            let section_inner = &line[1..line.len() - 1]; // remove [ ]
            let session_name_raw = if let Some(name) = section_inner.strip_prefix("Sessions\\") {
                name
            } else if let Some(name) = section_inner.strip_prefix("Sessions/") {
                name
            } else {
                i += 1;
                continue;
            };

            // URL-decode the session name (%20 -> space, etc.)
            let session_name = url_decode_simple(session_name_raw);

            // Read key=value pairs until the next section
            let mut kv: std::collections::HashMap<String, String> = std::collections::HashMap::new();
            i += 1;
            while i < lines.len() {
                let kv_line = lines[i].trim();
                if kv_line.starts_with('[') {
                    break; // next section
                }
                if let Some((key, value)) = kv_line.split_once('=') {
                    kv.insert(key.trim().to_string(), value.trim().to_string());
                }
                i += 1;
            }

            // Build profile if we have a HostName
            if let Some(host) = kv.get("HostName") {
                if host.is_empty() {
                    continue;
                }

                let protocol = kv
                    .get("FSProtocol")
                    .map(|p| winscp_protocol(p))
                    .unwrap_or("sftp");

                let port: Option<u16> = kv
                    .get("PortNumber")
                    .and_then(|p| p.parse().ok());

                let username = kv.get("UserName").cloned().filter(|u| !u.is_empty());
                let remote_path = kv.get("RemoteDirectory").cloned().filter(|d| !d.is_empty());

                // WinSCP password is obfuscated, not simply encoded. We store it
                // as-is and note in docs that re-entering the password is advised.
                let password = kv.get("Password").cloned().filter(|p| !p.is_empty());

                profiles.push(ImportedProfile {
                    name: session_name,
                    protocol: protocol.to_string(),
                    host: host.clone(),
                    port,
                    username,
                    password,
                    remote_path,
                    source_app: "winscp".to_string(),
                });
            }
        } else {
            i += 1;
        }
    }

    Ok(profiles)
}

/// Simple percent-decoding for WinSCP session names.
fn url_decode_simple(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(
                &s[i + 1..i + 3],
                16,
            ) {
                result.push(byte as char);
                i += 3;
                continue;
            }
        }
        result.push(bytes[i] as char);
        i += 1;
    }
    result
}

// ──────────────────────────────────────────────
// Transmit  (Favorites.xml - Apple XML plist)
// ──────────────────────────────────────────────

/// Map Transmit protocol strings to normalised protocol strings.
fn transmit_protocol(proto: &str) -> &'static str {
    let lower = proto.to_lowercase();
    if lower.contains("sftp") || lower.contains("ssh") {
        "sftp"
    } else if lower.contains("ftps") || lower.contains("ftp-ssl") || lower.contains("ftp-tls") {
        "ftps"
    } else if lower.contains("ftp") {
        "ftp"
    } else if lower.contains("webdav") || lower.contains("dav") {
        "webdav"
    } else if lower.contains("s3") || lower.contains("amazon") {
        "s3"
    } else if lower.contains("smb") {
        "smb"
    } else {
        "ftp"
    }
}

/// Parse a Transmit `Favorites.xml` (or `Favorites.plist`) file.
///
/// Transmit stores favorites as an array of dicts inside a plist:
/// ```xml
/// <plist version="1.0">
/// <dict>
///   <key>Favorites</key>
///   <array>
///     <dict>
///       <key>Name</key>
///       <string>My Server</string>
///       <key>Protocol</key>
///       <string>SFTP</string>
///       <key>Server</key>
///       <string>example.com</string>
///       <key>Port</key>
///       <integer>22</integer>
///       <key>UserName</key>
///       <string>admin</string>
///       <key>InitialRemotePath</key>
///       <string>/home/admin</string>
///     </dict>
///   </array>
/// </dict>
/// </plist>
/// ```
pub fn parse_transmit(path: &Path) -> Result<Vec<ImportedProfile>, AppError> {
    let content = std::fs::read_to_string(path)?;

    // Sanity check
    if !content.contains("<plist") {
        return Err(AppError::connection(
            "Not a valid Transmit Favorites file",
            "Select a Transmit Favorites.xml or Favorites.plist file.",
        ));
    }

    let mut profiles = Vec::new();

    // Find all <dict> blocks inside the <array> that follows the "Favorites" key.
    // Strategy: find `<key>Favorites</key>`, then extract the <array>...</array>,
    // then iterate its <dict> children.
    let favorites_section = extract_transmit_favorites_array(&content)
        .unwrap_or(content.as_str());

    // Split into individual <dict> blocks within the favorites array.
    let dict_blocks = xml_split_blocks(favorites_section, "dict");

    for dict in dict_blocks {
        // Transmit uses various key names across versions.
        let host = plist_string_value(dict, "Server")
            .or_else(|| plist_string_value(dict, "Hostname"))
            .or_else(|| plist_string_value(dict, "server"))
            .or_else(|| plist_string_value(dict, "hostname"))
            .map(xml_unescape);

        let host = match host {
            Some(h) if !h.is_empty() => h,
            _ => continue,
        };

        let name = plist_string_value(dict, "Name")
            .or_else(|| plist_string_value(dict, "Nickname"))
            .or_else(|| plist_string_value(dict, "name"))
            .or_else(|| plist_string_value(dict, "nickname"))
            .map(xml_unescape)
            .unwrap_or_else(|| host.clone());

        let protocol_raw = plist_string_value(dict, "Protocol")
            .or_else(|| plist_string_value(dict, "protocol"))
            .unwrap_or("FTP");
        let protocol = transmit_protocol(protocol_raw);

        let port: Option<u16> = plist_integer_value(dict, "Port")
            .or_else(|| plist_integer_value(dict, "port"))
            .and_then(|v| u16::try_from(v).ok());

        let username = plist_string_value(dict, "UserName")
            .or_else(|| plist_string_value(dict, "Username"))
            .or_else(|| plist_string_value(dict, "username"))
            .map(xml_unescape);

        let remote_path = plist_string_value(dict, "InitialRemotePath")
            .or_else(|| plist_string_value(dict, "RemotePath"))
            .or_else(|| plist_string_value(dict, "Path"))
            .or_else(|| plist_string_value(dict, "initialRemotePath"))
            .map(xml_unescape);

        profiles.push(ImportedProfile {
            name,
            protocol: protocol.to_string(),
            host,
            port,
            username,
            password: None, // Transmit stores passwords in the macOS Keychain
            remote_path,
            source_app: "transmit".to_string(),
        });
    }

    Ok(profiles)
}

/// Extract the `<array>...</array>` that follows `<key>Favorites</key>` in a Transmit plist.
fn extract_transmit_favorites_array(xml: &str) -> Option<&str> {
    let key_tag = "<key>Favorites</key>";
    let pos = xml.find(key_tag)?;
    let after = &xml[pos + key_tag.len()..];
    let after = after.trim_start();

    let array_start = after.find("<array>")?;
    let array_end = after.find("</array>")? + "</array>".len();
    Some(&after[array_start..array_end])
}

// ──────────────────────────────────────────────
// Auto-detection
// ──────────────────────────────────────────────

/// Detect the source format from file content/extension and parse accordingly.
///
/// Heuristics:
/// 1. If `source_hint` is provided, use it directly.
/// 2. File extension: `.duck` -> Cyberduck, `.ini` -> WinSCP, `.xml`/`.plist` -> inspect content.
/// 3. Content inspection: look for distinctive markers.
pub fn detect_and_parse(path: &Path) -> Result<Vec<ImportedProfile>, AppError> {
    let metadata = std::fs::metadata(path).map_err(|_| {
        AppError::not_found(format!("Import file not found: {}", path.display()))
    })?;

    // If it's a directory, assume Cyberduck bookmarks folder
    if metadata.is_dir() {
        return parse_cyberduck(path);
    }

    // Try extension first
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    match ext.as_str() {
        "duck" => return parse_cyberduck(path),
        "ini" => return parse_winscp(path),
        _ => {}
    }

    // Read content for inspection
    let content = std::fs::read_to_string(path).map_err(|e| {
        AppError::file_op(
            format!("Cannot read import file: {e}"),
            "Check file permissions.",
        )
    })?;

    // FileZilla: contains <FileZilla3 or <Server> with <Host>
    if content.contains("<FileZilla3") || (content.contains("<Server>") && content.contains("<Host>")) {
        return parse_filezilla(path);
    }

    // WinSCP: contains [Sessions\
    if content.contains("[Sessions\\") || content.contains("[Sessions/") {
        return parse_winscp(path);
    }

    // Cyberduck plist: contains Protocol + Hostname keys in a plist
    if content.contains("<plist") && content.contains("<key>Hostname</key>") {
        return parse_cyberduck(path);
    }

    // Transmit plist: contains Favorites key in a plist, or Server + Protocol keys
    if content.contains("<plist")
        && (content.contains("<key>Favorites</key>")
            || (content.contains("<key>Server</key>") && content.contains("<key>Protocol</key>")))
    {
        return parse_transmit(path);
    }

    Err(AppError::connection(
        "Could not determine the file format. Supported formats: FileZilla (sitemanager.xml), Cyberduck (.duck), WinSCP (.ini), Transmit (Favorites.xml).",
        "Specify the source application or choose a supported file.",
    ))
}

/// Parse with an explicit source application hint.
pub fn parse_with_hint(path: &Path, source_app: &str) -> Result<Vec<ImportedProfile>, AppError> {
    match source_app.to_lowercase().as_str() {
        "filezilla" => parse_filezilla(path),
        "cyberduck" => parse_cyberduck(path),
        "winscp" => parse_winscp(path),
        "transmit" => parse_transmit(path),
        other => Err(AppError::connection(
            format!("Unknown source application: {other}"),
            "Supported sources: filezilla, cyberduck, winscp, transmit.",
        )),
    }
}

// ──────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_xml_tag_value() {
        let xml = "<Host>example.com</Host>";
        assert_eq!(xml_tag_value(xml, "Host"), Some("example.com"));
        assert_eq!(xml_tag_value(xml, "Port"), None);
    }

    #[test]
    fn test_xml_tag_value_with_whitespace() {
        let xml = "<Port>  22  </Port>";
        assert_eq!(xml_tag_value(xml, "Port"), Some("22"));
    }

    #[test]
    fn test_xml_split_blocks() {
        let xml = "<root><Server><Host>a.com</Host></Server><Server><Host>b.com</Host></Server></root>";
        let blocks = xml_split_blocks(xml, "Server");
        assert_eq!(blocks.len(), 2);
        assert!(blocks[0].contains("a.com"));
        assert!(blocks[1].contains("b.com"));
    }

    #[test]
    fn test_xml_unescape() {
        assert_eq!(xml_unescape("a &amp; b"), "a & b");
        assert_eq!(xml_unescape("1 &lt; 2"), "1 < 2");
    }

    #[test]
    fn test_plist_string_value() {
        let xml = r#"<key>Protocol</key>
        <string>sftp</string>
        <key>Hostname</key>
        <string>example.com</string>"#;
        assert_eq!(plist_string_value(xml, "Protocol"), Some("sftp"));
        assert_eq!(plist_string_value(xml, "Hostname"), Some("example.com"));
        assert_eq!(plist_string_value(xml, "Missing"), None);
    }

    #[test]
    fn test_plist_integer_value() {
        let xml = r#"<key>Port</key>
        <integer>22</integer>"#;
        assert_eq!(plist_integer_value(xml, "Port"), Some(22));
    }

    #[test]
    fn test_url_decode_simple() {
        assert_eq!(url_decode_simple("My%20Server"), "My Server");
        assert_eq!(url_decode_simple("test%2Fpath"), "test/path");
        assert_eq!(url_decode_simple("plain"), "plain");
    }

    #[test]
    fn test_filezilla_protocol_mapping() {
        assert_eq!(filezilla_protocol("0"), "ftp");
        assert_eq!(filezilla_protocol("1"), "sftp");
        assert_eq!(filezilla_protocol("3"), "ftps");
        assert_eq!(filezilla_protocol("4"), "ftps");
        assert_eq!(filezilla_protocol("6"), "s3");
    }

    #[test]
    fn test_parse_filezilla_remote_dir() {
        assert_eq!(parse_filezilla_remote_dir("1 0 5 home 4 data"), "/home/data");
        assert_eq!(parse_filezilla_remote_dir(""), "/");
        assert_eq!(parse_filezilla_remote_dir("1 0"), "/");
    }

    #[test]
    fn test_parse_filezilla() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<FileZilla3 version="3.66.4">
    <Servers>
        <Server>
            <Host>sftp.example.com</Host>
            <Port>22</Port>
            <Protocol>1</Protocol>
            <Type>0</Type>
            <User>admin</User>
            <Pass encoding="base64">c2VjcmV0</Pass>
            <Name>Production Server</Name>
            <RemoteDir>1 0 4 home 5 admin</RemoteDir>
        </Server>
        <Server>
            <Host>ftp.example.com</Host>
            <Port>21</Port>
            <Protocol>0</Protocol>
            <Type>0</Type>
            <User>ftpuser</User>
            <Pass>plainpass</Pass>
            <Name>FTP Backup</Name>
        </Server>
    </Servers>
</FileZilla3>"#;

        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("sitemanager.xml");
        std::fs::write(&file_path, xml).unwrap();

        let profiles = parse_filezilla(&file_path).unwrap();
        assert_eq!(profiles.len(), 2);

        assert_eq!(profiles[0].name, "Production Server");
        assert_eq!(profiles[0].protocol, "sftp");
        assert_eq!(profiles[0].host, "sftp.example.com");
        assert_eq!(profiles[0].port, Some(22));
        assert_eq!(profiles[0].username, Some("admin".to_string()));
        assert_eq!(profiles[0].password, Some("secret".to_string()));
        assert_eq!(profiles[0].remote_path, Some("/home/admin".to_string()));
        assert_eq!(profiles[0].source_app, "filezilla");

        assert_eq!(profiles[1].name, "FTP Backup");
        assert_eq!(profiles[1].protocol, "ftp");
        assert_eq!(profiles[1].host, "ftp.example.com");
        assert_eq!(profiles[1].port, Some(21));
        assert_eq!(profiles[1].password, Some("plainpass".to_string()));
    }

    #[test]
    fn test_parse_filezilla_with_folders() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<FileZilla3>
    <Servers>
        <Folder expanded="1">
            <Server>
                <Host>nested.example.com</Host>
                <Port>2222</Port>
                <Protocol>1</Protocol>
                <User>nested_user</User>
                <Pass encoding="base64">cGFzcw==</Pass>
                <Name>Nested Server</Name>
            </Server>
            Folder Name
        </Folder>
        <Server>
            <Host>top.example.com</Host>
            <Port>22</Port>
            <Protocol>1</Protocol>
            <Name>Top-level</Name>
        </Server>
    </Servers>
</FileZilla3>"#;

        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("sitemanager.xml");
        std::fs::write(&file_path, xml).unwrap();

        let profiles = parse_filezilla(&file_path).unwrap();
        assert_eq!(profiles.len(), 2);
        assert_eq!(profiles[0].name, "Nested Server");
        assert_eq!(profiles[0].password, Some("pass".to_string()));
        assert_eq!(profiles[1].name, "Top-level");
    }

    #[test]
    fn test_parse_cyberduck() {
        let plist = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
    <dict>
        <key>Protocol</key>
        <string>sftp</string>
        <key>Nickname</key>
        <string>Dev Server</string>
        <key>Hostname</key>
        <string>dev.example.com</string>
        <key>Port</key>
        <integer>2222</integer>
        <key>Username</key>
        <string>devuser</string>
        <key>Path</key>
        <string>/var/www</string>
    </dict>
</plist>"#;

        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("dev-server.duck");
        std::fs::write(&file_path, plist).unwrap();

        let profiles = parse_cyberduck(&file_path).unwrap();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].name, "Dev Server");
        assert_eq!(profiles[0].protocol, "sftp");
        assert_eq!(profiles[0].host, "dev.example.com");
        assert_eq!(profiles[0].port, Some(2222));
        assert_eq!(profiles[0].username, Some("devuser".to_string()));
        assert_eq!(profiles[0].remote_path, Some("/var/www".to_string()));
        assert_eq!(profiles[0].password, None);
        assert_eq!(profiles[0].source_app, "cyberduck");
    }

    #[test]
    fn test_parse_cyberduck_directory() {
        let plist1 = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0">
    <dict>
        <key>Protocol</key>
        <string>sftp</string>
        <key>Hostname</key>
        <string>server1.com</string>
        <key>Nickname</key>
        <string>Server One</string>
    </dict>
</plist>"#;

        let plist2 = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0">
    <dict>
        <key>Protocol</key>
        <string>ftp</string>
        <key>Hostname</key>
        <string>server2.com</string>
        <key>Nickname</key>
        <string>Server Two</string>
    </dict>
</plist>"#;

        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("one.duck"), plist1).unwrap();
        std::fs::write(dir.path().join("two.duck"), plist2).unwrap();
        std::fs::write(dir.path().join("readme.txt"), "not a duck").unwrap();

        let profiles = parse_cyberduck(dir.path()).unwrap();
        assert_eq!(profiles.len(), 2);
    }

    #[test]
    fn test_parse_winscp() {
        let ini = r#"
[Configuration]
Version=5.21.5

[Sessions\Production%20Server]
HostName=prod.example.com
PortNumber=22
UserName=admin
FSProtocol=5
RemoteDirectory=/opt/app

[Sessions\FTP%20Backup]
HostName=backup.example.com
PortNumber=21
UserName=ftpuser
FSProtocol=2
Password=encrypted_placeholder

[Sessions\Default%20Settings]
"#;

        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("WinSCP.ini");
        std::fs::write(&file_path, ini).unwrap();

        let profiles = parse_winscp(&file_path).unwrap();
        assert_eq!(profiles.len(), 2);

        assert_eq!(profiles[0].name, "Production Server");
        assert_eq!(profiles[0].protocol, "sftp");
        assert_eq!(profiles[0].host, "prod.example.com");
        assert_eq!(profiles[0].port, Some(22));
        assert_eq!(profiles[0].username, Some("admin".to_string()));
        assert_eq!(profiles[0].remote_path, Some("/opt/app".to_string()));
        assert_eq!(profiles[0].source_app, "winscp");

        assert_eq!(profiles[1].name, "FTP Backup");
        assert_eq!(profiles[1].protocol, "ftp");
        assert_eq!(profiles[1].host, "backup.example.com");
        assert_eq!(profiles[1].port, Some(21));
        assert_eq!(profiles[1].password, Some("encrypted_placeholder".to_string()));
    }

    #[test]
    fn test_parse_winscp_skips_empty_hostname() {
        let ini = r#"
[Sessions\Empty%20Session]
HostName=
PortNumber=22

[Sessions\Valid%20Session]
HostName=valid.example.com
PortNumber=22
FSProtocol=5
"#;

        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("WinSCP.ini");
        std::fs::write(&file_path, ini).unwrap();

        let profiles = parse_winscp(&file_path).unwrap();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].name, "Valid Session");
    }

    #[test]
    fn test_parse_transmit() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN">
<plist version="1.0">
<dict>
    <key>Favorites</key>
    <array>
        <dict>
            <key>Name</key>
            <string>Web Server</string>
            <key>Protocol</key>
            <string>SFTP</string>
            <key>Server</key>
            <string>web.example.com</string>
            <key>Port</key>
            <integer>22</integer>
            <key>UserName</key>
            <string>webmaster</string>
            <key>InitialRemotePath</key>
            <string>/var/www/html</string>
        </dict>
        <dict>
            <key>Name</key>
            <string>S3 Bucket</string>
            <key>Protocol</key>
            <string>Amazon S3</string>
            <key>Server</key>
            <string>s3.amazonaws.com</string>
            <key>UserName</key>
            <string>AKIAIOSFODNN7EXAMPLE</string>
        </dict>
    </array>
</dict>
</plist>"#;

        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("Favorites.xml");
        std::fs::write(&file_path, xml).unwrap();

        let profiles = parse_transmit(&file_path).unwrap();
        assert_eq!(profiles.len(), 2);

        assert_eq!(profiles[0].name, "Web Server");
        assert_eq!(profiles[0].protocol, "sftp");
        assert_eq!(profiles[0].host, "web.example.com");
        assert_eq!(profiles[0].port, Some(22));
        assert_eq!(profiles[0].username, Some("webmaster".to_string()));
        assert_eq!(profiles[0].remote_path, Some("/var/www/html".to_string()));
        assert_eq!(profiles[0].source_app, "transmit");

        assert_eq!(profiles[1].name, "S3 Bucket");
        assert_eq!(profiles[1].protocol, "s3");
        assert_eq!(profiles[1].host, "s3.amazonaws.com");
        assert_eq!(profiles[1].username, Some("AKIAIOSFODNN7EXAMPLE".to_string()));
    }

    #[test]
    fn test_detect_and_parse_filezilla() {
        let xml = r#"<?xml version="1.0"?>
<FileZilla3>
    <Servers>
        <Server>
            <Host>auto.example.com</Host>
            <Port>22</Port>
            <Protocol>1</Protocol>
            <Name>Auto Detected</Name>
        </Server>
    </Servers>
</FileZilla3>"#;

        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("sitemanager.xml");
        std::fs::write(&file_path, xml).unwrap();

        let profiles = detect_and_parse(&file_path).unwrap();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].source_app, "filezilla");
    }

    #[test]
    fn test_detect_and_parse_duck_by_extension() {
        let plist = r#"<?xml version="1.0"?>
<plist version="1.0">
    <dict>
        <key>Protocol</key>
        <string>sftp</string>
        <key>Hostname</key>
        <string>duck.example.com</string>
    </dict>
</plist>"#;

        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("bookmark.duck");
        std::fs::write(&file_path, plist).unwrap();

        let profiles = detect_and_parse(&file_path).unwrap();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].source_app, "cyberduck");
    }

    #[test]
    fn test_detect_and_parse_winscp_by_extension() {
        let ini = r#"[Sessions\Test]
HostName=ini.example.com
PortNumber=22
FSProtocol=5
"#;

        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("WinSCP.ini");
        std::fs::write(&file_path, ini).unwrap();

        let profiles = detect_and_parse(&file_path).unwrap();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].source_app, "winscp");
    }

    #[test]
    fn test_detect_and_parse_unknown_format() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("random.txt");
        std::fs::write(&file_path, "This is not a bookmark file.").unwrap();

        let result = detect_and_parse(&file_path);
        assert!(result.is_err());
    }

    #[test]
    fn test_cyberduck_protocol_mapping() {
        assert_eq!(cyberduck_protocol("sftp"), "sftp");
        assert_eq!(cyberduck_protocol("ssh"), "sftp");
        assert_eq!(cyberduck_protocol("ftp"), "ftp");
        assert_eq!(cyberduck_protocol("ftp-ssl"), "ftps");
        assert_eq!(cyberduck_protocol("ftps"), "ftps");
        assert_eq!(cyberduck_protocol("dav"), "webdav");
        assert_eq!(cyberduck_protocol("davs"), "webdav");
        assert_eq!(cyberduck_protocol("s3"), "s3");
    }

    #[test]
    fn test_winscp_protocol_mapping() {
        assert_eq!(winscp_protocol("5"), "sftp");
        assert_eq!(winscp_protocol("0"), "ftp");
        assert_eq!(winscp_protocol("2"), "ftp");
        assert_eq!(winscp_protocol("7"), "ftps");
        assert_eq!(winscp_protocol("4"), "webdav");
        assert_eq!(winscp_protocol("1"), "s3");
    }

    #[test]
    fn test_transmit_protocol_mapping() {
        assert_eq!(transmit_protocol("SFTP"), "sftp");
        assert_eq!(transmit_protocol("FTP"), "ftp");
        assert_eq!(transmit_protocol("FTP-SSL"), "ftps");
        assert_eq!(transmit_protocol("WebDAV"), "webdav");
        assert_eq!(transmit_protocol("Amazon S3"), "s3");
        assert_eq!(transmit_protocol("SSH"), "sftp");
    }

    #[test]
    fn test_parse_with_hint() {
        let xml = r#"<?xml version="1.0"?>
<FileZilla3>
    <Servers>
        <Server>
            <Host>hint.example.com</Host>
            <Port>22</Port>
            <Protocol>1</Protocol>
            <Name>Hint Test</Name>
        </Server>
    </Servers>
</FileZilla3>"#;

        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("connections.xml");
        std::fs::write(&file_path, xml).unwrap();

        let profiles = parse_with_hint(&file_path, "filezilla").unwrap();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].name, "Hint Test");

        // Unknown hint should error
        let err = parse_with_hint(&file_path, "unknown_app");
        assert!(err.is_err());
    }

    #[test]
    fn test_filezilla_xml_entities() {
        let xml = r#"<?xml version="1.0"?>
<FileZilla3>
    <Servers>
        <Server>
            <Host>example.com</Host>
            <Port>22</Port>
            <Protocol>1</Protocol>
            <Name>AT&amp;T Server</Name>
            <User>admin</User>
        </Server>
    </Servers>
</FileZilla3>"#;

        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("sitemanager.xml");
        std::fs::write(&file_path, xml).unwrap();

        let profiles = parse_filezilla(&file_path).unwrap();
        assert_eq!(profiles[0].name, "AT&T Server");
    }

    #[test]
    fn test_cyberduck_no_nickname_falls_back_to_hostname() {
        let plist = r#"<?xml version="1.0"?>
<plist version="1.0">
    <dict>
        <key>Protocol</key>
        <string>ftp</string>
        <key>Hostname</key>
        <string>fallback.example.com</string>
    </dict>
</plist>"#;

        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("no-nickname.duck");
        std::fs::write(&file_path, plist).unwrap();

        let profiles = parse_cyberduck(&file_path).unwrap();
        assert_eq!(profiles[0].name, "fallback.example.com");
    }
}
