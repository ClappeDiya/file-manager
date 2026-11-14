//! `ufop mcp` — a Model Context Protocol server over stdio (newline-delimited
//! JSON-RPC 2.0) that gives an AI client file tools confined to `--root`
//! directories. Read-only unless `--allow-write`. Every tool call is logged
//! (`--log-file`); stdout carries only protocol messages. The protocol and
//! tools are `ufop_core::mcp` (shared with `ufop-server`'s `/mcp`).

use clap::Args;
use std::path::PathBuf;
use ufop_core::mcp::{Roots, Server};

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

pub async fn execute(args: McpArgs) -> Result<u8, Box<dyn std::error::Error>> {
    let server = Server { roots: Roots::new(&args.roots)?, allow_write: args.allow_write, max_read_bytes: args.max_read_bytes.max(1) };
    tracing::info!(target: "ufop::mcp", roots = ?server.roots.list().iter().map(|r| r.path.display().to_string()).collect::<Vec<_>>(), allow_write = server.allow_write, "MCP server started");
    tokio::task::spawn_blocking(move || server.run(std::io::stdin().lock(), std::io::stdout().lock())).await??;
    Ok(0)
}
