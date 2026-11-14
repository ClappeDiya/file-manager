//! `ufop backup bench` (codec, cipher, KDF, chunker and hash benchmarks)
//! and `ufop backup debug …` (read-only expert inspection of a repository).

use super::backup::{emit, parse_bytes, with_repo, RepoArgs};
use crate::output;
use crate::OutputFormat;
use clap::{Args, Subcommand};
use std::path::PathBuf;
use ufop_core::backup::bench::{self, BenchOptions, BenchReport, BenchSection};
use ufop_core::backup::db::BackupDb;
use ufop_core::backup::inspect::{self, BlockFilter, VolumeFilter};
use ufop_core::backup::lock::LockKind;
use ufop_core::error::AppError;

/// `ufop backup bench` flags.
#[derive(Args, Debug, Clone)]
pub struct BenchArgs {
    /// Read the sample from this file or folder instead of generating it
    #[arg(long)]
    pub sample: Option<PathBuf>,
    /// Sample size to generate, or the most to read from --sample (e.g. 16M)
    #[arg(long, default_value = "16M")]
    pub size: String,
    /// Seed for generated data and the edited copy used for deduplication
    #[arg(long, default_value_t = 1)]
    pub seed: u64,
    /// Runs per measurement; the fastest counts
    #[arg(long, default_value_t = 3)]
    pub repeat: u32,
    /// Worker threads for compression, encryption and hashing
    #[arg(long, default_value_t = 1)]
    pub threads: usize,
    /// Block size for compression/encryption and the base of the chunker variants
    #[arg(long, default_value = "100K")]
    pub block_size: String,
    /// Only these sections: compression, crypto, kdf, chunkers, hashes (comma-separated)
    #[arg(long, value_delimiter = ',')]
    pub only: Vec<String>,
    /// Target time for the suggested Argon2id parameters (0 = no suggestion)
    #[arg(long, default_value_t = 1000)]
    pub kdf_target_ms: u64,
    /// Memory for the suggested Argon2id parameters, in KiB
    #[arg(long, default_value_t = 65536)]
    pub kdf_memory_kib: u32,
    /// Print only the recommended `ufop backup run` flags
    #[arg(long)]
    pub print_options: bool,
}

impl BenchArgs {
    pub fn options(&self) -> Result<BenchOptions, AppError> {
        let sections = self
            .only
            .iter()
            .filter(|s| !s.trim().is_empty())
            .map(|s| BenchSection::parse(s).ok_or_else(|| AppError::validation(format!("Unknown benchmark section '{s}' (compression, crypto, kdf, chunkers, hashes)"))))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(BenchOptions {
            size: parse_bytes(&self.size)? as usize,
            seed: self.seed,
            sample_path: self.sample.clone(),
            repeat: self.repeat,
            threads: self.threads,
            block_size: parse_bytes(&self.block_size)? as usize,
            sections,
            kdf_target_ms: self.kdf_target_ms,
            kdf_memory_kib: self.kdf_memory_kib,
            ..Default::default()
        })
    }
}

fn mb(v: f64) -> String {
    format!("{v:>9.1} MB/s")
}

/// Human-readable benchmark tables.
pub fn bench_text(r: &BenchReport) -> String {
    let mut o = format!(
        "Sample: {} ({} bytes{}), block {} bytes, best of {} run(s), {} thread(s)\n",
        r.sample.source,
        r.sample.bytes,
        r.sample.seed.map(|s| format!(", seed {s}")).unwrap_or_default(),
        r.block_size,
        r.repeat,
        r.threads
    );
    if !r.compression.is_empty() {
        o.push_str("\nCompression (ranked: space saved weighed against speed)\n  #  codec        ratio    compress       decompress     flags\n");
        for c in &r.compression {
            let flags = if c.zip_only { "--volume-format zip --zip-method bzip2".to_string() } else if c.codec == "stored" { "--compression-level 0".into() } else { format!("--block-codec {} --compression-level {}", c.codec, c.level) };
            o.push_str(&format!("{:>3}  {:<11} {:>6.1}%  {}  {}  {flags}\n", c.rank, c.name, c.ratio * 100.0, mb(c.compress_mb_per_sec), mb(c.decompress_mb_per_sec)));
        }
    }
    if !r.ciphers.is_empty() {
        o.push_str("\nEncryption\n  cipher              seal           open\n");
        for c in &r.ciphers {
            o.push_str(&format!("  {:<18} {}  {}\n", c.cipher.as_str(), mb(c.seal_mb_per_sec), mb(c.open_mb_per_sec)));
        }
    }
    if !r.kdf.is_empty() || r.kdf_suggestion.is_some() {
        o.push_str("\nArgon2id key derivation\n");
        for k in r.kdf.iter().chain(r.kdf_suggestion.iter()) {
            o.push_str(&format!("  {:<12} {:>5} MiB  t={:<2} p={:<2} {:>8.0} ms\n", k.name, k.params.memory_kib / 1024, k.params.iterations, k.params.parallelism, k.millis));
        }
    }
    if !r.chunkers.is_empty() {
        o.push_str("\nChunkers (dedup = share of an edited copy found again)\n  chunker               speed         blocks   avg      p10      p50      p90      dedup\n");
        for c in &r.chunkers {
            o.push_str(&format!("  {:<20} {}  {:>7}  {:>7.0}  {:>7}  {:>7}  {:>7}  {:>5.1}%\n", c.name, mb(c.mb_per_sec), c.chunks, c.avg_size, c.p10, c.p50, c.p90, c.dedup_ratio * 100.0));
        }
    }
    if !r.hashes.is_empty() {
        o.push_str("\nHashes\n");
        for h in &r.hashes {
            o.push_str(&format!("  {:<8} {}\n", format!("{:?}", h.algorithm).to_lowercase(), mb(h.mb_per_sec)));
        }
    }
    if !r.recommended.reasons.is_empty() {
        o.push_str("\nRecommended for a new backup:\n");
        for why in &r.recommended.reasons {
            o.push_str(&format!("  - {why}\n"));
        }
        o.push_str(&format!("  {}\n", r.backup_run_command()));
    }
    for w in &r.warnings {
        o.push_str(&format!("warning: {w}\n"));
    }
    o
}

pub fn bench(args: &BenchArgs, format: &OutputFormat) -> Result<u8, AppError> {
    let opts = args.options()?;
    let ctx = super::backup::progress_ctx(format);
    if matches!(format, OutputFormat::Human) && !args.print_options {
        eprintln!("Benchmarking (this takes a little while)…");
    }
    let r = bench::run(&opts, &ctx)?;
    if args.print_options {
        if matches!(format, OutputFormat::Human) {
            println!("{}", r.backup_run_flags().join(" "));
        } else {
            output::print_data(format, &serde_json::json!({ "flags": r.backup_run_flags(), "command": r.backup_run_command(), "recommended": r.recommended }));
        }
    } else if matches!(format, OutputFormat::Human) {
        print!("{}", bench_text(&r));
    } else {
        output::print_data(format, &r);
    }
    Ok(0)
}

/// `ufop backup debug …`
#[derive(Subcommand, Debug, Clone)]
pub enum DebugCommands {
    /// The repository header (repo.json) decoded: format, features, chunker, codec, cipher
    Header {
        #[command(flatten)]
        repo: RepoArgs,
    },
    /// Files on the destination with kind, size and shard folder
    Volumes {
        #[command(flatten)]
        repo: RepoArgs,
        /// dblock | dindex | dlist | header | lock | other
        #[arg(long)]
        kind: Option<String>,
        /// Name prefix after the repository prefix (e.g. `b1f`)
        #[arg(long)]
        name_prefix: Option<String>,
        /// Smallest size (e.g. 1M)
        #[arg(long)]
        min_size: Option<String>,
        /// Largest size (e.g. 50M)
        #[arg(long)]
        max_size: Option<String>,
    },
    /// Counts and bytes per kind, blocks, deduplication and per-version use
    Stats {
        #[command(flatten)]
        repo: RepoArgs,
    },
    /// Blocks with size, volume and (with --details or --volume) stored size and codec
    Blocks {
        #[command(flatten)]
        repo: RepoArgs,
        /// Only blocks of this version (0 = newest)
        #[arg(long)]
        version: Option<usize>,
        /// Only blocks in this dblock volume
        #[arg(long)]
        volume: Option<String>,
        /// Only blocks whose hash starts with this
        #[arg(long)]
        hash_prefix: Option<String>,
        /// Download the volumes for stored size and codec
        #[arg(long)]
        details: bool,
        /// Show at most N blocks (0 = all)
        #[arg(long, default_value_t = 0)]
        max_blocks: usize,
    },
    /// Fetch and decode one block
    Block {
        #[command(flatten)]
        repo: RepoArgs,
        hash: String,
        /// The stored (still compressed) bytes instead of the decompressed data
        #[arg(long, conflicts_with = "decompressed")]
        raw: bool,
        /// The decompressed data (default)
        #[arg(long)]
        decompressed: bool,
        /// Write the bytes to this file (default: only describe the block)
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// A decoded index (dindex) volume
    Index {
        #[command(flatten)]
        repo: RepoArgs,
        name: String,
    },
    /// The decoded file list (dlist) of a version, with its metadata
    Fileset {
        #[command(flatten)]
        repo: RepoArgs,
        /// 0 = newest
        version: usize,
    },
    /// Who holds the repository lock: the writer and the readers
    Sessions {
        #[command(flatten)]
        repo: RepoArgs,
    },
    /// The local job database (same as `ufop backup db info`)
    Db { dbpath: PathBuf },
}

fn size_opt(s: &Option<String>) -> Result<Option<u64>, AppError> {
    s.as_deref().map(parse_bytes).transpose()
}

fn ok(v: Result<(), Box<dyn std::error::Error>>) -> Result<(), AppError> {
    v.map_err(|e| AppError::internal(e.to_string()))
}

const OP: &str = "debug";

pub fn debug(cmd: DebugCommands, format: &OutputFormat) -> Result<u8, AppError> {
    let human = matches!(format, OutputFormat::Human);
    match cmd {
        DebugCommands::Header { repo } => {
            let h = inspect::header(repo.backend().as_ref(), &repo.prefix)?;
            ok(emit(format, "Repository header", &h))?;
            Ok(if h.unknown_features.is_empty() { 0 } else { 1 })
        }
        DebugCommands::Volumes { repo, kind, name_prefix, min_size, max_size } => {
            let f = VolumeFilter { kind, name_prefix, min_size: size_opt(&min_size)?, max_size: size_opt(&max_size)? };
            with_repo(&repo, LockKind::Shared, OP, |r, _| {
                let v = inspect::volumes(r, &f)?;
                if human {
                    for x in &v {
                        println!("{:<7} {:>12}  {:<4} {:<9} {}", x.kind, x.size, x.shard.as_deref().unwrap_or("-"), x.state.as_deref().unwrap_or("-"), x.name);
                    }
                    eprintln!("{} file(s), {} bytes", v.len(), v.iter().map(|x| x.size).sum::<u64>());
                } else {
                    output::print_data(format, &v);
                }
                Ok(0)
            })
        }
        DebugCommands::Stats { repo } => with_repo(&repo, LockKind::Shared, OP, |r, _| {
            ok(emit(format, "Repository statistics", &inspect::stats(r)?))?;
            Ok(0)
        }),
        DebugCommands::Blocks { repo, version, volume, hash_prefix, details, max_blocks } => with_repo(&repo, LockKind::Shared, OP, |r, _| {
            let b = inspect::blocks(r, &BlockFilter { version, volume, hash_prefix, details, limit: max_blocks })?;
            if human {
                for x in &b {
                    let stored = x.stored_size.map(|s| s.to_string()).unwrap_or_else(|| "-".into());
                    println!("{}  {:>9}  {:>9}  {:<8} {}", x.hash, x.size, stored, x.codec.as_deref().unwrap_or("-"), x.volume);
                }
                eprintln!("{} block(s)", b.len());
            } else {
                output::print_data(format, &b);
            }
            Ok(0)
        }),
        DebugCommands::Block { repo, hash, raw, decompressed: _, out } => with_repo(&repo, LockKind::Shared, OP, |r, _| {
            let b = inspect::block(r, &hash, raw)?;
            if let Some(p) = &out {
                std::fs::write(p, &b.data).map_err(|e| AppError::file_op(format!("Cannot write {}: {e}", p.display()), "Check the output folder exists and is writable."))?;
            }
            let mut v = serde_json::to_value(&b).map_err(|e| AppError::internal(e.to_string()))?;
            v["written_bytes"] = serde_json::json!(out.as_ref().map(|_| b.data.len()));
            v["form"] = serde_json::json!(if raw { "raw" } else { "decompressed" });
            ok(emit(format, "Block", &v))?;
            Ok(if b.hash_ok { 0 } else { 2 })
        }),
        DebugCommands::Index { repo, name } => with_repo(&repo, LockKind::Shared, OP, |r, _| {
            ok(emit(format, "Index volume", &inspect::index(r, &name)?))?;
            Ok(0)
        }),
        DebugCommands::Fileset { repo, version } => with_repo(&repo, LockKind::Shared, OP, |r, _| {
            let (name, fl) = inspect::fileset(r, version)?;
            let mut v = serde_json::to_value(&fl).map_err(|e| AppError::internal(e.to_string()))?;
            v["volume"] = serde_json::json!(name);
            ok(emit(format, "File list", &v))?;
            Ok(0)
        }),
        DebugCommands::Sessions { repo } => {
            let s = inspect::sessions(repo.backend().as_ref(), &repo.prefix)?;
            if human {
                eprintln!("{}", s.summary.clone().unwrap_or_else(|| "The repository is not in use".into()));
            }
            ok(emit(format, "Repository sessions", &s))?;
            Ok(0)
        }
        DebugCommands::Db { dbpath } => {
            let i = BackupDb::inspect(&dbpath)?;
            ok(emit(format, "Database", &i))?;
            Ok(if i.integrity_ok { 0 } else { 2 })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Parser, Debug)]
    struct T {
        #[command(subcommand)]
        cmd: super::super::backup::BackupCommands,
    }

    fn parse(args: &[&str]) -> super::super::backup::BackupCommands {
        let mut v = vec!["t"];
        v.extend_from_slice(args);
        T::try_parse_from(v).unwrap().cmd
    }

    #[test]
    fn bench_flags_map_to_options() {
        match parse(&["bench", "--size", "1M", "--block-size", "64K", "--only", "compression,chunkers", "--repeat", "2", "--threads", "3", "--seed", "9"]) {
            super::super::backup::BackupCommands::Bench { args } => {
                let o = args.options().unwrap();
                assert_eq!((o.size, o.block_size, o.repeat, o.threads, o.seed), (1 << 20, 64 << 10, 2, 3, 9));
                assert_eq!(o.sections, vec![BenchSection::Compression, BenchSection::Chunkers]);
            }
            other => panic!("{other:?}"),
        }
        match parse(&["bench", "--only", "nonsense"]) {
            super::super::backup::BackupCommands::Bench { args } => assert!(args.options().is_err()),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn bench_runs_and_prints_tables() {
        let o = BenchOptions { size: 64 * 1024, repeat: 1, block_size: 8 * 1024, kdf_target_ms: 0, kdf_presets: vec!["interactive".into()], sections: vec![BenchSection::Compression, BenchSection::Crypto, BenchSection::Chunkers, BenchSection::Hashes], ..Default::default() };
        let r = bench::run(&o, &ufop_core::backup::OpContext::new()).unwrap();
        let t = bench_text(&r);
        for s in ["Compression", "zstd-3", "Encryption", "chacha20_poly1305", "Chunkers", "fastcdc-8K", "Hashes", "sha512", "ufop backup run --block-codec"] {
            assert!(t.contains(s), "{s} missing:\n{t}");
        }
    }

    #[test]
    fn debug_subcommands_parse() {
        match parse(&["debug", "volumes", "--dest", "/tmp/x", "--kind", "dblock", "--name-prefix", "b1", "--min-size", "1K"]) {
            super::super::backup::BackupCommands::Debug { cmd: DebugCommands::Volumes { kind, name_prefix, min_size, .. } } => {
                assert_eq!((kind.as_deref(), name_prefix.as_deref()), (Some("dblock"), Some("b1")));
                assert_eq!(size_opt(&min_size).unwrap(), Some(1024));
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(parse(&["debug", "block", "--dest", "/x", "abcd", "--raw", "--out", "/tmp/b"]), super::super::backup::BackupCommands::Debug { cmd: DebugCommands::Block { raw: true, .. } }));
        assert!(matches!(parse(&["debug", "fileset", "--dest", "/x", "2"]), super::super::backup::BackupCommands::Debug { cmd: DebugCommands::Fileset { version: 2, .. } }));
        assert!(matches!(parse(&["debug", "db", "/x.sqlite"]), super::super::backup::BackupCommands::Debug { cmd: DebugCommands::Db { .. } }));
        for c in ["header", "stats", "sessions", "blocks"] {
            assert!(matches!(parse(&["debug", c, "--dest", "/x"]), super::super::backup::BackupCommands::Debug { .. }));
        }
    }
}
