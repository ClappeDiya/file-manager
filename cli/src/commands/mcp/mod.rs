//! `ufop mcp` — a Model Context Protocol server over stdio (newline-delimited
//! JSON-RPC 2.0) that gives an AI client file tools confined to `--root`
//! directories. Read-only unless `--allow-write`. Every tool call is logged
//! (`--log-file`); stdout carries only protocol messages.

mod roots;
mod tools;

use clap::Args;
use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::path::PathBuf;

pub use roots::Roots;

/// Protocol revisions this server speaks, newest first.
const PROTOCOL_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

#[derive(Args, Debug)]
pub struct McpArgs {
    /// Directory the client may use (repeat for several); nothing outside is reachable
    #[arg(long = "root", required = true)]
    pub roots: Vec<PathBuf>,
    /// Also offer tools that create, change, move and delete files
    #[arg(long)]
    pub allow_write: bool,
    /// Largest read per call, in bytes
    #[arg(long, default_value_t = 4 * 1024 * 1024)]
    pub max_read_bytes: u64,
}

pub struct Server {
    pub roots: Roots,
    pub allow_write: bool,
    pub max_read_bytes: u64,
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

impl Server {
    /// Handle one JSON-RPC message; `None` for notifications.
    pub fn handle(&self, msg: &Value) -> Option<Value> {
        let id = msg.get("id").cloned();
        let method = msg.get("method").and_then(Value::as_str);
        let (Some(id), Some(method)) = (id.clone(), method) else {
            // A notification (no id) or a response from the client: nothing to answer.
            return match (id, method) {
                (Some(id), None) if msg.get("result").is_none() && msg.get("error").is_none() => Some(error(id, -32600, "invalid request")),
                _ => None,
            };
        };
        if msg.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            return Some(error(id, -32600, "invalid request: jsonrpc must be \"2.0\""));
        }
        let params = msg.get("params").cloned().unwrap_or(json!({}));
        let result = match method {
            "initialize" => {
                let asked = params.get("protocolVersion").and_then(Value::as_str).unwrap_or(PROTOCOL_VERSIONS[0]);
                let version = PROTOCOL_VERSIONS.iter().find(|v| **v == asked).copied().unwrap_or(PROTOCOL_VERSIONS[0]);
                json!({
                    "protocolVersion": version,
                    "capabilities": { "tools": { "listChanged": false } },
                    "serverInfo": { "name": "ufop", "title": "UFOP files", "version": env!("CARGO_PKG_VERSION") },
                    "instructions": format!(
                        "File tools confined to these roots: {}. Paths are `<root>:relative/path`, absolute paths inside a root, or relative to the first root.{}",
                        self.roots.list().iter().map(|r| r.name.as_str()).collect::<Vec<_>>().join(", "),
                        if self.allow_write { "" } else { " This server is read-only." }
                    ),
                })
            }
            "ping" => json!({}),
            "tools/list" => json!({ "tools": tools::definitions(self.allow_write) }),
            "tools/call" => {
                let name = params.get("name").and_then(Value::as_str).unwrap_or_default();
                let args = params.get("arguments").cloned().unwrap_or(json!({}));
                if !tools::exists(name, self.allow_write) {
                    return Some(error(id, -32602, &format!("unknown tool: {name}")));
                }
                let out = tools::call(self, name, &args);
                match &out {
                    Ok(_) => tracing::info!(target: "ufop::mcp", tool = name, args = %args, "tool call"),
                    Err(e) => tracing::warn!(target: "ufop::mcp", tool = name, args = %args, error = %e, "tool call failed"),
                }
                match out {
                    Ok(v) => json!({ "content": [{ "type": "text", "text": serde_json::to_string_pretty(&v).unwrap_or_default() }], "structuredContent": v, "isError": false }),
                    // Tool failures are results the model can read, not protocol errors.
                    Err(e) => json!({ "content": [{ "type": "text", "text": e }], "isError": true }),
                }
            }
            "resources/list" => json!({ "resources": [] }),
            "prompts/list" => json!({ "prompts": [] }),
            _ => return Some(error(id, -32601, &format!("method not found: {method}"))),
        };
        Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
    }

    /// Serve until the input closes.
    pub fn run(&self, input: impl BufRead, mut output: impl Write) -> std::io::Result<()> {
        for line in input.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let reply = match serde_json::from_str::<Value>(&line) {
                Ok(Value::Array(batch)) => {
                    let replies: Vec<Value> = batch.iter().filter_map(|m| self.handle(m)).collect();
                    (!replies.is_empty()).then_some(Value::Array(replies))
                }
                Ok(msg) => self.handle(&msg),
                Err(e) => Some(error(Value::Null, -32700, &format!("parse error: {e}"))),
            };
            if let Some(r) = reply {
                writeln!(output, "{r}")?;
                output.flush()?;
            }
        }
        Ok(())
    }
}

pub async fn execute(args: McpArgs) -> Result<u8, Box<dyn std::error::Error>> {
    let server = Server { roots: Roots::new(&args.roots)?, allow_write: args.allow_write, max_read_bytes: args.max_read_bytes.max(1) };
    tracing::info!(target: "ufop::mcp", roots = ?server.roots.list().iter().map(|r| r.path.display().to_string()).collect::<Vec<_>>(), allow_write = server.allow_write, "MCP server started");
    tokio::task::spawn_blocking(move || server.run(std::io::stdin().lock(), std::io::stdout().lock())).await??;
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(dir: &std::path::Path, allow_write: bool) -> Server {
        Server { roots: Roots::new(&[dir.to_path_buf()]).unwrap(), allow_write, max_read_bytes: 1024 }
    }

    fn call(s: &Server, name: &str, args: Value) -> Value {
        s.handle(&json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": name, "arguments": args } })).unwrap()["result"].clone()
    }

    #[test]
    fn protocol_handshake_and_errors() {
        let d = tempfile::tempdir().unwrap();
        let s = server(d.path(), false);
        let input = [
            json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2025-03-26", "capabilities": {}, "clientInfo": { "name": "t", "version": "1" } } }).to_string(),
            json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }).to_string(),
            json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }).to_string(),
            "not json".into(),
            json!({ "jsonrpc": "2.0", "id": 3, "method": "nope" }).to_string(),
            json!({ "jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": { "name": "write_file", "arguments": {} } }).to_string(),
            json!([{ "jsonrpc": "2.0", "id": 5, "method": "ping" }]).to_string(),
        ]
        .join("\n");
        let mut out = Vec::new();
        s.run(std::io::Cursor::new(input), &mut out).unwrap();
        let replies: Vec<Value> = String::from_utf8(out).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(replies.len(), 6, "the notification gets no reply");
        assert_eq!(replies[0]["result"]["protocolVersion"], "2025-03-26");
        assert!(replies[0]["result"]["instructions"].as_str().unwrap().contains("read-only"));
        let names: Vec<&str> = replies[1]["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"read_text_file") && !names.contains(&"write_file"), "read-only: {names:?}");
        assert_eq!(replies[2]["error"]["code"], -32700);
        assert_eq!(replies[3]["error"]["code"], -32601);
        assert_eq!(replies[4]["error"]["code"], -32602, "write tools do not exist without --allow-write");
        assert_eq!(replies[5][0]["result"], json!({}));
    }

    #[test]
    fn read_tools_stay_inside_the_root() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("root");
        std::fs::create_dir_all(root.join("docs/deep")).unwrap();
        std::fs::write(root.join("docs/a.txt"), "hello world").unwrap();
        std::fs::write(root.join("docs/deep/b.md"), "# b").unwrap();
        std::fs::write(root.join("bin.dat"), [0u8, 159, 146, 150]).unwrap();
        std::fs::write(d.path().join("outside.txt"), "secret").unwrap();
        let s = server(&root, false);

        let r = call(&s, "list_directory", json!({ "path": "root:docs" }));
        assert_eq!(r["isError"], false);
        let entries = r["structuredContent"]["entries"].as_array().unwrap();
        assert_eq!(entries.iter().map(|e| e["path"].as_str().unwrap()).collect::<Vec<_>>(), ["root:docs/a.txt", "root:docs/deep"]);
        let all = call(&s, "list_directory", json!({ "path": "", "recursive": true }));
        assert_eq!(all["structuredContent"]["entries"].as_array().unwrap().len(), 5);

        let t = call(&s, "read_text_file", json!({ "path": "docs/a.txt", "offset": 6 }));
        assert_eq!(t["structuredContent"]["text"], "world");
        let big = call(&s, "read_text_file", json!({ "path": "docs/a.txt", "max_bytes": 5 }));
        assert_eq!(big["structuredContent"], json!({ "path": "root:docs/a.txt", "text": "hello", "encoding": "utf8", "offset": 0, "bytes": 5, "size": 11, "truncated": true }));
        assert!(call(&s, "read_text_file", json!({ "path": "bin.dat" }))["content"][0]["text"].as_str().unwrap().contains("base64"));
        assert_eq!(call(&s, "read_text_file", json!({ "path": "bin.dat", "encoding": "base64" }))["structuredContent"]["text"], "AJ+Slg==");

        let found = call(&s, "search_files", json!({ "path": "root:", "pattern": "*.md" }));
        assert_eq!(found["structuredContent"]["matches"], json!(["root:docs/deep/b.md"]));
        let info = call(&s, "get_file_info", json!({ "path": "docs/a.txt" }));
        assert_eq!(info["structuredContent"]["size"], 11);
        assert_eq!(call(&s, "checksum", json!({ "path": "docs/a.txt" }))["structuredContent"]["hash"], "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9");

        for (tool, args) in [
            ("read_text_file", json!({ "path": "../outside.txt" })),
            ("read_text_file", json!({ "path": d.path().join("outside.txt").to_string_lossy() })),
            ("list_directory", json!({ "path": "/" })),
            ("get_file_info", json!({ "path": "docs/../../outside.txt" })),
        ] {
            let r = call(&s, tool, args.clone());
            assert_eq!(r["isError"], true, "{tool} {args}");
            assert!(r["content"][0]["text"].as_str().unwrap().contains("outside"), "{tool} {args}: {r}");
        }
        assert_eq!(call(&s, "list_roots", json!({}))["structuredContent"]["roots"][0]["name"], "root");
    }

    #[test]
    fn write_tools_when_allowed() {
        let d = tempfile::tempdir().unwrap();
        let root = d.path().join("w");
        std::fs::create_dir_all(&root).unwrap();
        let s = server(&root, true);
        assert_eq!(call(&s, "write_file", json!({ "path": "new/dir/f.txt", "content": "v1" }))["isError"], false);
        assert_eq!(std::fs::read_to_string(root.join("new/dir/f.txt")).unwrap(), "v1");
        assert!(call(&s, "write_file", json!({ "path": "new/dir/f.txt", "content": "v2" }))["content"][0]["text"].as_str().unwrap().contains("exists"));
        call(&s, "write_file", json!({ "path": "new/dir/f.txt", "content": "djI=", "encoding": "base64", "overwrite": true }));
        assert_eq!(std::fs::read_to_string(root.join("new/dir/f.txt")).unwrap(), "v2");
        assert_eq!(call(&s, "copy_file", json!({ "source": "new/dir/f.txt", "destination": "c.txt" }))["isError"], false);
        assert_eq!(call(&s, "move", json!({ "source": "c.txt", "destination": "m.txt" }))["isError"], false);
        assert!(!root.join("c.txt").exists() && root.join("m.txt").exists());
        assert_eq!(call(&s, "create_directory", json!({ "path": "made/here" }))["isError"], false);
        assert!(call(&s, "delete", json!({ "path": "new" }))["content"][0]["text"].as_str().unwrap().contains("recursive"));
        assert_eq!(call(&s, "delete", json!({ "path": "new", "recursive": true }))["isError"], false);
        assert!(!root.join("new").exists());
        assert!(call(&s, "delete", json!({ "path": "w:" }))["content"][0]["text"].as_str().unwrap().contains("is a root"));
        assert!(call(&s, "write_file", json!({ "path": "../escape.txt", "content": "x" }))["isError"] == true);
        assert!(call(&s, "move", json!({ "source": "m.txt", "destination": "../m.txt" }))["isError"] == true);
        assert!(!d.path().join("escape.txt").exists() && !d.path().join("m.txt").exists());
    }
}
