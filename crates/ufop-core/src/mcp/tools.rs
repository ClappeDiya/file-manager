//! The MCP tools: definitions (name, description, JSON Schema, hints) and
//! their implementations. Every path goes through `Roots`.

use super::Server;
use base64::Engine;
use serde_json::{json, Value};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::time::UNIX_EPOCH;

const MAX_LIST: u64 = 10_000;
const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

struct Tool {
    name: &'static str,
    description: &'static str,
    /// JSON Schema `properties` and `required`.
    properties: fn() -> Value,
    required: &'static [&'static str],
    write: bool,
    destructive: bool,
}

const PATH: &str = "`<root>:relative/path`, an absolute path inside a root, or a path relative to the first root";

fn tools() -> Vec<Tool> {
    vec![
        Tool { name: "list_roots", description: "List the directories this server may use.", properties: || json!({}), required: &[], write: false, destructive: false },
        Tool {
            name: "list_directory",
            description: "List a directory (name, type, size, modified time). Symbolic links are listed, not followed.",
            properties: || json!({ "path": { "type": "string", "description": PATH }, "recursive": { "type": "boolean", "default": false }, "max_entries": { "type": "integer", "minimum": 1, "maximum": MAX_LIST, "default": 1000 } }),
            required: &["path"],
            write: false,
            destructive: false,
        },
        Tool {
            name: "get_file_info",
            description: "Type, size, times and permissions of a file or directory.",
            properties: || json!({ "path": { "type": "string", "description": PATH } }),
            required: &["path"],
            write: false,
            destructive: false,
        },
        Tool {
            name: "read_text_file",
            description: "Read a file (or a byte range of it) as UTF-8 text, or as base64 with encoding=base64.",
            properties: || json!({ "path": { "type": "string", "description": PATH }, "offset": { "type": "integer", "minimum": 0, "default": 0 }, "max_bytes": { "type": "integer", "minimum": 1 }, "encoding": { "type": "string", "enum": ["utf8", "base64"], "default": "utf8" } }),
            required: &["path"],
            write: false,
            destructive: false,
        },
        Tool {
            name: "search_files",
            description: "Find files under a directory whose name matches a glob pattern (e.g. *.pdf, report-202?-*.xlsx).",
            properties: || json!({ "path": { "type": "string", "description": PATH }, "pattern": { "type": "string" }, "max_results": { "type": "integer", "minimum": 1, "maximum": MAX_LIST, "default": 200 } }),
            required: &["path", "pattern"],
            write: false,
            destructive: false,
        },
        Tool {
            name: "checksum",
            description: "Hash a file (sha256, sha1 or md5).",
            properties: || json!({ "path": { "type": "string", "description": PATH }, "algorithm": { "type": "string", "enum": ["sha256", "sha1", "md5"], "default": "sha256" } }),
            required: &["path"],
            write: false,
            destructive: false,
        },
        Tool {
            name: "write_file",
            description: "Create a file (and missing parent directories) with text or base64 content; replaces an existing file only with overwrite=true. Written atomically.",
            properties: || json!({ "path": { "type": "string", "description": PATH }, "content": { "type": "string" }, "encoding": { "type": "string", "enum": ["utf8", "base64"], "default": "utf8" }, "overwrite": { "type": "boolean", "default": false } }),
            required: &["path", "content"],
            write: true,
            destructive: true,
        },
        Tool {
            name: "create_directory",
            description: "Create a directory and any missing parents.",
            properties: || json!({ "path": { "type": "string", "description": PATH } }),
            required: &["path"],
            write: true,
            destructive: false,
        },
        Tool {
            name: "copy_file",
            description: "Copy a file; replaces an existing destination only with overwrite=true.",
            properties: || json!({ "source": { "type": "string", "description": PATH }, "destination": { "type": "string", "description": PATH }, "overwrite": { "type": "boolean", "default": false } }),
            required: &["source", "destination"],
            write: true,
            destructive: true,
        },
        Tool {
            name: "move",
            description: "Move or rename a file or directory within the roots; replaces an existing file only with overwrite=true.",
            properties: || json!({ "source": { "type": "string", "description": PATH }, "destination": { "type": "string", "description": PATH }, "overwrite": { "type": "boolean", "default": false } }),
            required: &["source", "destination"],
            write: true,
            destructive: true,
        },
        Tool {
            name: "delete",
            description: "Delete a file, or a directory (non-empty ones only with recursive=true). Roots cannot be deleted.",
            properties: || json!({ "path": { "type": "string", "description": PATH }, "recursive": { "type": "boolean", "default": false } }),
            required: &["path"],
            write: true,
            destructive: true,
        },
    ]
}

pub fn definitions(allow_write: bool) -> Vec<Value> {
    tools()
        .into_iter()
        .filter(|t| allow_write || !t.write)
        .map(|t| {
            json!({
                "name": t.name,
                "description": t.description,
                "inputSchema": { "type": "object", "properties": (t.properties)(), "required": t.required, "additionalProperties": false },
                "annotations": { "readOnlyHint": !t.write, "destructiveHint": t.destructive, "idempotentHint": !t.write || t.name == "create_directory", "openWorldHint": false },
            })
        })
        .collect()
}

/// Whether `name` creates, changes, moves or deletes files.
pub fn is_write_tool(name: &str) -> bool {
    tools().iter().any(|t| t.name == name && t.write)
}

/// Hex digest of a file (`sha256`, `sha1` or `md5`), read in 64 KiB blocks.
pub fn hash_file(path: &Path, algorithm: &str) -> Result<String, String> {
    use sha2::Digest;
    fn stream<D: sha2::Digest>(mut f: std::fs::File, mut h: D) -> Result<String, String> {
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let n = f.read(&mut buf).map_err(io)?;
            if n == 0 {
                break;
            }
            h.update(&buf[..n]);
        }
        Ok(hex_encode(&h.finalize()))
    }
    let f = std::fs::File::open(path).map_err(io)?;
    match algorithm {
        "md5" => stream(f, md5::Md5::new()),
        "sha1" => stream(f, sha1::Sha1::new()),
        "sha256" => stream(f, sha2::Sha256::new()),
        other => Err(format!("Unsupported algorithm: {other} (use md5, sha1 or sha256)")),
    }
}

fn hex_encode(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn exists(name: &str, allow_write: bool) -> bool {
    tools().iter().any(|t| t.name == name && (allow_write || !t.write))
}

fn s<'a>(a: &'a Value, k: &str) -> Result<&'a str, String> {
    a.get(k).and_then(Value::as_str).ok_or_else(|| format!("'{k}' is required"))
}
fn b(a: &Value, k: &str) -> bool {
    a.get(k).and_then(Value::as_bool).unwrap_or(false)
}
fn n(a: &Value, k: &str, default: u64, max: u64) -> u64 {
    a.get(k).and_then(Value::as_u64).unwrap_or(default).clamp(1, max)
}
fn io(e: std::io::Error) -> String {
    e.to_string()
}

fn secs(t: std::io::Result<std::time::SystemTime>) -> Value {
    t.ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).and_then(|d| chrono::DateTime::from_timestamp(d.as_secs() as i64, 0)).map(|d| json!(d.to_rfc3339())).unwrap_or(Value::Null)
}

fn kind(m: &std::fs::Metadata) -> &'static str {
    if m.file_type().is_symlink() {
        "symlink"
    } else if m.is_dir() {
        "directory"
    } else {
        "file"
    }
}

fn entry(srv: &Server, p: &Path) -> Value {
    let m = std::fs::symlink_metadata(p).ok();
    json!({
        "name": p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        "path": srv.roots.display(p),
        "type": m.as_ref().map(kind).unwrap_or("unknown"),
        "size": m.as_ref().filter(|m| m.is_file()).map(|m| m.len()),
        "modified": m.as_ref().map(|m| secs(m.modified())).unwrap_or(Value::Null),
    })
}

/// Walk `dir` (not following symlinks), calling `f` for each entry in name order until it returns false.
fn walk(dir: &Path, recursive: bool, f: &mut dyn FnMut(&Path) -> bool) -> Result<(), String> {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let mut items: Vec<_> = std::fs::read_dir(&d).map_err(io)?.filter_map(Result::ok).map(|e| e.path()).collect();
        items.sort();
        let mut subdirs = Vec::new();
        for p in items {
            if !f(&p) {
                return Ok(());
            }
            if recursive && std::fs::symlink_metadata(&p).map(|m| m.is_dir()).unwrap_or(false) {
                subdirs.push(p);
            }
        }
        stack.extend(subdirs.into_iter().rev());
    }
    Ok(())
}

/// Replace `dest` only when allowed; refuses to overwrite a directory.
fn check_dest(dest: &Path, overwrite: bool, input: &str) -> Result<(), String> {
    match std::fs::symlink_metadata(dest) {
        Ok(m) if m.is_dir() => Err(format!("{input}: is a directory")),
        Ok(_) if !overwrite => Err(format!("{input}: already exists (pass overwrite=true to replace it)")),
        _ => Ok(()),
    }
}

/// Write via a temporary file in the same directory, then rename into place.
fn write_atomic(dest: &Path, bytes: &[u8]) -> Result<(), String> {
    let dir = dest.parent().ok_or("no parent directory")?;
    std::fs::create_dir_all(dir).map_err(io)?;
    let tmp = dir.join(format!(".{}.ufop-mcp-{}", dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), uuid::Uuid::new_v4().simple()));
    std::fs::write(&tmp, bytes).map_err(io)?;
    std::fs::rename(&tmp, dest).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })
}

pub fn call(srv: &Server, name: &str, a: &Value) -> Result<Value, String> {
    let roots = &srv.roots;
    match name {
        "list_roots" => Ok(json!({ "roots": roots.list().iter().map(|r| json!({ "name": r.name, "path": r.path.display().to_string(), "uri": format!("{}:", r.name) })).collect::<Vec<_>>(), "writable": srv.allow_write })),
        "list_directory" => {
            let dir = roots.resolve(s(a, "path")?, true)?;
            if !dir.is_dir() {
                return Err(format!("{}: not a directory", s(a, "path")?));
            }
            let max = n(a, "max_entries", 1000, MAX_LIST) as usize;
            let mut entries = Vec::new();
            let mut truncated = false;
            walk(&dir, b(a, "recursive"), &mut |p| {
                if entries.len() >= max {
                    truncated = true;
                    return false;
                }
                entries.push(entry(srv, p));
                true
            })?;
            Ok(json!({ "path": roots.display(&dir), "entries": entries, "truncated": truncated }))
        }
        "get_file_info" => {
            let p = roots.resolve(s(a, "path")?, true)?;
            let m = std::fs::metadata(&p).map_err(io)?;
            #[cfg(unix)]
            let mode = {
                use std::os::unix::fs::PermissionsExt;
                json!(format!("{:o}", m.permissions().mode() & 0o7777))
            };
            #[cfg(not(unix))]
            let mode = Value::Null;
            Ok(json!({ "path": roots.display(&p), "type": kind(&m), "size": m.len(), "modified": secs(m.modified()), "created": secs(m.created()), "accessed": secs(m.accessed()), "readonly": m.permissions().readonly(), "mode": mode }))
        }
        "read_text_file" => {
            let input = s(a, "path")?;
            let p = roots.resolve(input, true)?;
            if !p.is_file() {
                return Err(format!("{input}: not a file"));
            }
            let size = std::fs::metadata(&p).map_err(io)?.len();
            let offset = a.get("offset").and_then(Value::as_u64).unwrap_or(0);
            let max = n(a, "max_bytes", srv.max_read_bytes, srv.max_read_bytes);
            let mut f = std::fs::File::open(&p).map_err(io)?;
            f.seek(SeekFrom::Start(offset)).map_err(io)?;
            let mut buf = Vec::new();
            f.take(max).read_to_end(&mut buf).map_err(io)?;
            let base64 = a.get("encoding").and_then(Value::as_str) == Some("base64");
            let text = if base64 {
                B64.encode(&buf)
            } else {
                match String::from_utf8(buf.clone()) {
                    Ok(t) => t,
                    // A range may end inside a multi-byte character: keep the valid prefix.
                    Err(e) if e.utf8_error().error_len().is_none() && offset + (buf.len() as u64) < size => {
                        let valid = e.utf8_error().valid_up_to();
                        buf.truncate(valid);
                        String::from_utf8(buf.clone()).map_err(|e| e.to_string())?
                    }
                    Err(_) => return Err(format!("{input}: not UTF-8 text; read it with encoding=base64")),
                }
            };
            Ok(json!({ "path": roots.display(&p), "text": text, "encoding": if base64 { "base64" } else { "utf8" }, "offset": offset, "bytes": buf.len(), "size": size, "truncated": offset + (buf.len() as u64) < size }))
        }
        "search_files" => {
            let dir = roots.resolve(s(a, "path")?, true)?;
            let pattern = glob::Pattern::new(s(a, "pattern")?).map_err(|e| format!("pattern: {e}"))?;
            let max = n(a, "max_results", 200, MAX_LIST) as usize;
            let opts = glob::MatchOptions { case_sensitive: false, ..Default::default() };
            let mut matches = Vec::new();
            let mut truncated = false;
            walk(&dir, true, &mut |p| {
                if p.file_name().is_some_and(|n| pattern.matches_with(&n.to_string_lossy(), opts)) {
                    if matches.len() >= max {
                        truncated = true;
                        return false;
                    }
                    matches.push(roots.display(p));
                }
                true
            })?;
            Ok(json!({ "matches": matches, "truncated": truncated }))
        }
        "checksum" => {
            let p = roots.resolve(s(a, "path")?, true)?;
            let algorithm = a.get("algorithm").and_then(Value::as_str).unwrap_or("sha256");
            let hash = hash_file(&p, algorithm)?;
            Ok(json!({ "path": roots.display(&p), "algorithm": algorithm, "hash": hash }))
        }
        "write_file" => {
            let input = s(a, "path")?;
            let p = roots.resolve_target(input)?;
            check_dest(&p, b(a, "overwrite"), input)?;
            let content = s(a, "content")?;
            let bytes = if a.get("encoding").and_then(Value::as_str) == Some("base64") { B64.decode(content).map_err(|e| format!("content: {e}"))? } else { content.as_bytes().to_vec() };
            write_atomic(&p, &bytes)?;
            Ok(json!({ "path": roots.display(&p), "bytes": bytes.len() }))
        }
        "create_directory" => {
            let p = roots.resolve(s(a, "path")?, false)?;
            std::fs::create_dir_all(&p).map_err(io)?;
            Ok(json!({ "path": roots.display(&p) }))
        }
        "copy_file" | "move" => {
            let src_in = s(a, "source")?;
            let dest_in = s(a, "destination")?;
            let src = roots.resolve(src_in, true)?;
            if roots.list().iter().any(|r| r.path == src) {
                return Err(format!("{src_in}: is a root"));
            }
            let dest = roots.resolve_target(dest_in)?;
            check_dest(&dest, b(a, "overwrite"), dest_in)?;
            if dest.starts_with(&src) {
                return Err(format!("{dest_in}: is inside {src_in}"));
            }
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).map_err(io)?;
            }
            if name == "copy_file" {
                if !src.is_file() {
                    return Err(format!("{src_in}: not a file"));
                }
                std::fs::copy(&src, &dest).map_err(io)?;
            } else {
                std::fs::rename(&src, &dest).map_err(|e| format!("{e} (moves across file systems are not supported; copy and delete instead)"))?;
            }
            Ok(json!({ "source": roots.display(&src), "destination": roots.display(&dest) }))
        }
        "delete" => {
            let input = s(a, "path")?;
            let p = roots.resolve_target(input)?;
            let m = std::fs::symlink_metadata(&p).map_err(|e| format!("{input}: {e}"))?;
            if m.is_dir() {
                if b(a, "recursive") {
                    std::fs::remove_dir_all(&p).map_err(io)?;
                } else {
                    std::fs::remove_dir(&p).map_err(|e| format!("{input}: {e} (pass recursive=true to delete a non-empty directory)"))?;
                }
            } else {
                std::fs::remove_file(&p).map_err(io)?;
            }
            Ok(json!({ "deleted": roots.display(&p) }))
        }
        _ => Err(format!("unknown tool: {name}")),
    }
}
