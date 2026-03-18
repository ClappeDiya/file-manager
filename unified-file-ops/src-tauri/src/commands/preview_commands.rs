//! Preview Engine commands (T-061).
//!
//! Sandboxed preview for images, PDFs, text/code (30+ lang syntax highlight),
//! Markdown rendered, audio waveform, video thumbnail, archive contents, EXIF.
//! Render <500ms for files <10MB. Large files (>50MB): metadata only.

use crate::core::error::AppError;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Maximum file size for full preview (10 MB).
const MAX_PREVIEW_SIZE: u64 = 10 * 1024 * 1024;

/// Files larger than this get metadata-only preview (50 MB).
const METADATA_ONLY_SIZE: u64 = 50 * 1024 * 1024;

/// Maximum text preview lines.
const MAX_TEXT_LINES: usize = 500;

/// Preview result returned to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreviewResult {
    pub path: String,
    pub file_name: String,
    pub file_size: u64,
    pub mime_type: String,
    pub preview_type: PreviewType,
    pub content: Option<String>,
    pub metadata: FileMetadata,
    pub is_metadata_only: bool,
    pub render_hint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreviewType {
    Image,
    Pdf,
    Text,
    Code,
    Markdown,
    Audio,
    Video,
    Archive,
    Exif,
    Binary,
    TooLarge,
    Unsupported,
}

/// Basic file metadata always returned.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileMetadata {
    pub size: u64,
    pub size_human: String,
    pub created: Option<String>,
    pub modified: Option<String>,
    pub accessed: Option<String>,
    pub is_readonly: bool,
    pub is_symlink: bool,
    pub permissions: Option<String>,
    pub exif: Option<ExifData>,
}

/// EXIF metadata for images.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExifData {
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub camera_make: Option<String>,
    pub camera_model: Option<String>,
    pub date_taken: Option<String>,
    pub iso: Option<u32>,
    pub exposure: Option<String>,
    pub f_number: Option<String>,
    pub focal_length: Option<String>,
    pub gps_lat: Option<f64>,
    pub gps_lon: Option<f64>,
}

/// Archive contents listing for preview.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchivePreviewEntry {
    pub name: String,
    pub size: u64,
    pub is_dir: bool,
    pub compressed_size: Option<u64>,
}

/// Language detection result for syntax highlighting.
fn detect_language(ext: &str) -> &'static str {
    match ext.to_lowercase().as_str() {
        "rs" => "rust",
        "py" => "python",
        "js" | "mjs" | "cjs" => "javascript",
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "tsx",
        "jsx" => "jsx",
        "html" | "htm" => "html",
        "css" => "css",
        "scss" | "sass" => "scss",
        "less" => "less",
        "json" => "json",
        "yaml" | "yml" => "yaml",
        "toml" => "toml",
        "xml" | "svg" | "xsl" | "xslt" => "xml",
        "md" | "markdown" => "markdown",
        "sh" | "bash" | "zsh" => "bash",
        "ps1" | "psm1" => "powershell",
        "bat" | "cmd" => "batch",
        "c" => "c",
        "h" => "c",
        "cpp" | "cc" | "cxx" | "c++" => "cpp",
        "hpp" | "hxx" | "h++" => "cpp",
        "cs" => "csharp",
        "java" => "java",
        "kt" | "kts" => "kotlin",
        "swift" => "swift",
        "go" => "go",
        "rb" => "ruby",
        "php" => "php",
        "r" | "rmd" => "r",
        "sql" => "sql",
        "lua" => "lua",
        "dart" => "dart",
        "scala" => "scala",
        "groovy" => "groovy",
        "pl" | "pm" => "perl",
        "zig" => "zig",
        "nim" => "nim",
        "ex" | "exs" => "elixir",
        "erl" | "hrl" => "erlang",
        "hs" | "lhs" => "haskell",
        "ml" | "mli" => "ocaml",
        "fs" | "fsi" | "fsx" => "fsharp",
        "clj" | "cljs" | "cljc" => "clojure",
        "vue" => "vue",
        "svelte" => "svelte",
        "dockerfile" => "dockerfile",
        "makefile" => "makefile",
        "cmake" => "cmake",
        "graphql" | "gql" => "graphql",
        "proto" => "protobuf",
        "tf" | "tfvars" => "hcl",
        "ini" | "cfg" | "conf" => "ini",
        "log" => "log",
        "txt" | "text" => "plaintext",
        "csv" | "tsv" => "csv",
        _ => "plaintext",
    }
}

/// Determine the MIME type from extension.
fn mime_from_ext(ext: &str) -> &'static str {
    match ext.to_lowercase().as_str() {
        // Images
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        "tiff" | "tif" => "image/tiff",
        "heic" | "heif" => "image/heic",
        "avif" => "image/avif",
        // PDF
        "pdf" => "application/pdf",
        // Audio
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",
        "flac" => "audio/flac",
        "aac" => "audio/aac",
        "m4a" => "audio/mp4",
        "wma" => "audio/x-ms-wma",
        // Video
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mkv" => "video/x-matroska",
        "avi" => "video/x-msvideo",
        "mov" => "video/quicktime",
        "wmv" => "video/x-ms-wmv",
        "flv" => "video/x-flv",
        // Archives
        "zip" => "application/zip",
        "tar" => "application/x-tar",
        "gz" | "tgz" => "application/gzip",
        "7z" => "application/x-7z-compressed",
        "rar" => "application/vnd.rar",
        "bz2" => "application/x-bzip2",
        "xz" => "application/x-xz",
        // Text / Code
        "txt" | "log" | "csv" | "tsv" => "text/plain",
        "html" | "htm" => "text/html",
        "css" => "text/css",
        "js" | "mjs" => "text/javascript",
        "json" => "application/json",
        "xml" => "application/xml",
        "yaml" | "yml" => "text/yaml",
        "md" | "markdown" => "text/markdown",
        _ => "application/octet-stream",
    }
}

/// Generate a preview for a file path.
#[tauri::command]
pub async fn preview_file(path: String) -> Result<PreviewResult, AppError> {
    let file_path = Path::new(&path);
    if !file_path.exists() {
        return Err(AppError::not_found(format!("File not found: {path}")));
    }

    let meta = std::fs::metadata(&path).map_err(|e| {
        AppError::file_op(format!("Cannot read metadata: {e}"), "Check file permissions")
    })?;

    let file_size = meta.len();
    let file_name = file_path
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_default();

    let ext = file_path
        .extension()
        .map(|e| e.to_string_lossy().to_string())
        .unwrap_or_default();

    let mime_type = mime_from_ext(&ext).to_string();

    // Build metadata
    let file_meta = FileMetadata {
        size: file_size,
        size_human: format_bytes(file_size),
        created: meta.created().ok().map(|t| {
            let dt: chrono::DateTime<chrono::Utc> = t.into();
            dt.to_rfc3339()
        }),
        modified: meta.modified().ok().map(|t| {
            let dt: chrono::DateTime<chrono::Utc> = t.into();
            dt.to_rfc3339()
        }),
        accessed: meta.accessed().ok().map(|t| {
            let dt: chrono::DateTime<chrono::Utc> = t.into();
            dt.to_rfc3339()
        }),
        is_readonly: meta.permissions().readonly(),
        is_symlink: file_path.is_symlink(),
        permissions: {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                Some(format!("{:o}", meta.permissions().mode()))
            }
            #[cfg(not(unix))]
            {
                None
            }
        },
        exif: None,
    };

    // Large file: metadata only
    if file_size > METADATA_ONLY_SIZE {
        return Ok(PreviewResult {
            path,
            file_name,
            file_size,
            mime_type,
            preview_type: PreviewType::TooLarge,
            content: None,
            metadata: file_meta,
            is_metadata_only: true,
            render_hint: "metadata_only".into(),
        });
    }

    // Determine preview type based on MIME
    let (preview_type, content, render_hint) = if mime_type.starts_with("image/") {
        // Images: return path for frontend to render
        (PreviewType::Image, None, format!("image:{ext}"))
    } else if mime_type == "application/pdf" {
        (PreviewType::Pdf, None, "pdf".into())
    } else if mime_type.starts_with("audio/") {
        (PreviewType::Audio, None, format!("audio:{ext}"))
    } else if mime_type.starts_with("video/") {
        (PreviewType::Video, None, format!("video:{ext}"))
    } else if mime_type == "text/markdown" {
        // Markdown: return raw content for rendered display
        let text = read_text_preview(&path, file_size)?;
        (PreviewType::Markdown, Some(text), "markdown".into())
    } else if is_archive_ext(&ext) {
        // Archives: list contents
        let listing = list_archive_contents(&path, &ext)?;
        (PreviewType::Archive, Some(listing), "archive".into())
    } else if is_text_or_code(&ext, &mime_type) {
        // Text/code: return content with language hint
        let lang = detect_language(&ext);
        let text = read_text_preview(&path, file_size)?;
        if lang == "plaintext" {
            (PreviewType::Text, Some(text), format!("text:{lang}"))
        } else {
            (PreviewType::Code, Some(text), format!("code:{lang}"))
        }
    } else {
        // Binary/unknown
        let hex_preview = if file_size <= MAX_PREVIEW_SIZE {
            read_hex_preview(&path)?
        } else {
            None
        };
        (PreviewType::Binary, hex_preview, "binary".into())
    };

    Ok(PreviewResult {
        path,
        file_name,
        file_size,
        mime_type,
        preview_type,
        content,
        metadata: file_meta,
        is_metadata_only: false,
        render_hint,
    })
}

/// Get EXIF data for an image file.
#[tauri::command]
pub async fn preview_get_exif(path: String) -> Result<ExifData, AppError> {
    let file_path = Path::new(&path);
    if !file_path.exists() {
        return Err(AppError::not_found(format!("File not found: {path}")));
    }

    // Basic EXIF extraction from JPEG/TIFF headers
    // In production, use the `kamadak-exif` or `rexif` crate
    // For now, return basic image dimensions from PNG/JPEG headers
    let ext = file_path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    let (width, height) = match ext.as_str() {
        "png" => read_png_dimensions(&path)?,
        "jpg" | "jpeg" => read_jpeg_dimensions(&path)?,
        _ => (None, None),
    };

    Ok(ExifData {
        width,
        height,
        camera_make: None,
        camera_model: None,
        date_taken: None,
        iso: None,
        exposure: None,
        f_number: None,
        focal_length: None,
        gps_lat: None,
        gps_lon: None,
    })
}

fn read_text_preview(path: &str, size: u64) -> Result<String, AppError> {
    if size > MAX_PREVIEW_SIZE {
        // Read only first portion
        let mut file = std::fs::File::open(path).map_err(|e| {
            AppError::file_op(format!("Cannot open: {e}"), "Check permissions")
        })?;
        let mut buf = vec![0u8; MAX_PREVIEW_SIZE as usize];
        use std::io::Read;
        let n = file.read(&mut buf).map_err(|e| {
            AppError::file_op(format!("Read error: {e}"), "Check file access")
        })?;
        let text = String::from_utf8_lossy(&buf[..n]).to_string();

        // Truncate to MAX_TEXT_LINES
        let truncated: String = text.lines().take(MAX_TEXT_LINES).collect::<Vec<_>>().join("\n");
        Ok(truncated)
    } else {
        let content = std::fs::read_to_string(path).map_err(|e| {
            AppError::file_op(format!("Cannot read text: {e}"), "File may be binary")
        })?;
        let truncated: String = content.lines().take(MAX_TEXT_LINES).collect::<Vec<_>>().join("\n");
        Ok(truncated)
    }
}

fn read_hex_preview(path: &str) -> Result<Option<String>, AppError> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).map_err(|e| {
        AppError::file_op(format!("Cannot open: {e}"), "Check permissions")
    })?;
    let mut buf = [0u8; 256]; // First 256 bytes
    let n = file.read(&mut buf).unwrap_or(0);
    if n == 0 {
        return Ok(None);
    }

    let mut hex_lines = Vec::new();
    for chunk in buf[..n].chunks(16) {
        let hex: Vec<String> = chunk.iter().map(|b| format!("{b:02x}")).collect();
        let ascii: String = chunk
            .iter()
            .map(|&b| if b.is_ascii_graphic() || b == b' ' { b as char } else { '.' })
            .collect();
        hex_lines.push(format!("{}  {}", hex.join(" "), ascii));
    }

    Ok(Some(hex_lines.join("\n")))
}

fn is_text_or_code(ext: &str, mime: &str) -> bool {
    if mime.starts_with("text/") {
        return true;
    }
    matches!(
        ext.to_lowercase().as_str(),
        "rs" | "py" | "js" | "ts" | "tsx" | "jsx" | "html" | "css" | "scss" | "less"
        | "json" | "yaml" | "yml" | "toml" | "xml" | "md" | "sh" | "bash" | "zsh"
        | "c" | "h" | "cpp" | "cc" | "hpp" | "cs" | "java" | "kt" | "swift"
        | "go" | "rb" | "php" | "r" | "sql" | "lua" | "dart" | "scala"
        | "pl" | "pm" | "zig" | "nim" | "ex" | "exs" | "erl" | "hs" | "ml"
        | "fs" | "clj" | "vue" | "svelte" | "graphql" | "proto" | "tf"
        | "ini" | "cfg" | "conf" | "log" | "txt" | "csv" | "tsv" | "env"
        | "gitignore" | "dockerignore" | "editorconfig" | "ps1" | "bat" | "cmd"
        | "makefile" | "cmake" | "dockerfile" | "groovy"
    )
}

fn is_archive_ext(ext: &str) -> bool {
    matches!(
        ext.to_lowercase().as_str(),
        "zip" | "tar" | "gz" | "tgz" | "7z" | "rar" | "bz2" | "xz"
    )
}

fn list_archive_contents(path: &str, ext: &str) -> Result<String, AppError> {
    let output = match ext.to_lowercase().as_str() {
        "zip" => std::process::Command::new("zipinfo")
            .args(["-1", path])
            .output(),
        "tar" => std::process::Command::new("tar")
            .args(["-tf", path])
            .output(),
        "gz" | "tgz" => std::process::Command::new("tar")
            .args(["-tzf", path])
            .output(),
        _ => return Ok("(Archive listing not available for this format)".into()),
    };

    match output {
        Ok(out) => Ok(String::from_utf8_lossy(&out.stdout).to_string()),
        Err(_) => Ok("(Could not list archive contents)".into()),
    }
}

fn read_png_dimensions(path: &str) -> Result<(Option<u32>, Option<u32>), AppError> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).map_err(|e| {
        AppError::file_op(format!("Cannot open: {e}"), "Check permissions")
    })?;
    let mut header = [0u8; 24];
    file.read_exact(&mut header).map_err(|e| {
        AppError::file_op(format!("Read error: {e}"), "File too small")
    })?;

    // PNG signature: 89 50 4E 47 0D 0A 1A 0A
    if &header[0..8] != b"\x89PNG\r\n\x1a\n" {
        return Ok((None, None));
    }

    // IHDR chunk starts at offset 8, width at 16, height at 20
    let width = u32::from_be_bytes([header[16], header[17], header[18], header[19]]);
    let height = u32::from_be_bytes([header[20], header[21], header[22], header[23]]);

    Ok((Some(width), Some(height)))
}

fn read_jpeg_dimensions(path: &str) -> Result<(Option<u32>, Option<u32>), AppError> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).map_err(|e| {
        AppError::file_op(format!("Cannot open: {e}"), "Check permissions")
    })?;

    let mut buf = vec![0u8; 65536.min(
        std::fs::metadata(path).map(|m| m.len() as usize).unwrap_or(0),
    )];
    let n = file.read(&mut buf).unwrap_or(0);
    if n < 4 {
        return Ok((None, None));
    }

    // Search for SOF0 marker (0xFF 0xC0) or SOF2 (0xFF 0xC2)
    let mut i = 0;
    while i + 9 < n {
        if buf[i] == 0xFF && (buf[i + 1] == 0xC0 || buf[i + 1] == 0xC2) {
            let height = u16::from_be_bytes([buf[i + 5], buf[i + 6]]) as u32;
            let width = u16::from_be_bytes([buf[i + 7], buf[i + 8]]) as u32;
            return Ok((Some(width), Some(height)));
        }
        i += 1;
    }

    Ok((None, None))
}

fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}
