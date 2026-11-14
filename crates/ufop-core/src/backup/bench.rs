//! `ufop backup bench`: measure the engine's codecs, ciphers, key
//! derivation, chunkers and hashes on sample data and suggest settings.
//!
//! Every measurement goes through the code the engine uses for real
//! backups — [`encode_block`]/[`decode_block`] for block compression,
//! [`Bzip2`] for ZIP volumes, [`VolumeCipher::seal`]/[`open`](VolumeCipher::open)
//! for volumes, [`VolumeCipher::derive`] for Argon2id, [`ChunkerSpec::build`]
//! with [`for_each_chunk`] for chunking and [`hash_bytes`] for hashes — so
//! the numbers describe what a backup will see on this machine.
//!
//! Timings are the best of `repeat` runs (like Kopia's benchmarks); the
//! sample is either generated from a seed (mixed text, structured records
//! and random bytes, so compression ratios are realistic) or read from a
//! file or folder.

use super::chunker::{for_each_chunk, ChunkerSpec};
use super::crypto::{CipherKind, KdfParams, VolumeCipher};
use super::modules::{Bzip2, CompressionModule};
use super::volume::{decode_block, encode_block, zstd_level};
use super::{hash_bytes, BlockCodec, HashAlgorithm, VolumeFormat};
use crate::error::AppError;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Parts of the benchmark.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BenchSection {
    Compression,
    Crypto,
    Kdf,
    Chunkers,
    Hashes,
}

impl BenchSection {
    pub const ALL: [BenchSection; 5] = [Self::Compression, Self::Crypto, Self::Kdf, Self::Chunkers, Self::Hashes];

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "compression" | "codecs" => Self::Compression,
            "crypto" | "ciphers" | "encryption" => Self::Crypto,
            "kdf" | "argon2" => Self::Kdf,
            "chunkers" | "chunker" | "splitters" => Self::Chunkers,
            "hashes" | "hash" => Self::Hashes,
            _ => return None,
        })
    }
}

/// What to measure and on which data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BenchOptions {
    /// Sample bytes to generate, or the most to read from `sample_path`.
    pub size: usize,
    /// Seed for generated data (same seed, same data).
    pub seed: u64,
    /// A file or folder to read the sample from instead of generating it.
    pub sample_path: Option<PathBuf>,
    /// Runs per measurement; the fastest counts.
    pub repeat: u32,
    /// Worker threads for compression, encryption and hashing.
    pub threads: usize,
    /// Block size the sample is cut into for compression and encryption,
    /// and the base size of the chunker variants.
    pub block_size: usize,
    /// Sections to run (empty = all).
    pub sections: Vec<BenchSection>,
    /// Target time for the suggested Argon2id parameters.
    pub kdf_target_ms: u64,
    /// Memory for the suggested Argon2id parameters (KiB).
    pub kdf_memory_kib: u32,
    pub kdf_parallelism: u32,
    /// Named Argon2id presets to time (empty = interactive, default, sensitive).
    pub kdf_presets: Vec<String>,
}

impl Default for BenchOptions {
    fn default() -> Self {
        Self {
            size: 16 << 20,
            seed: 1,
            sample_path: None,
            repeat: 3,
            threads: 1,
            block_size: super::BackupOptions::default().block_size,
            sections: Vec::new(),
            kdf_target_ms: 1000,
            kdf_memory_kib: KdfParams::default().memory_kib,
            kdf_parallelism: KdfParams::default().parallelism,
            kdf_presets: Vec::new(),
        }
    }
}

impl BenchOptions {
    fn runs(&self, s: BenchSection) -> bool {
        self.sections.is_empty() || self.sections.contains(&s)
    }

    pub fn validate(&self) -> Result<(), AppError> {
        if self.size == 0 || self.size > 4 << 30 {
            return Err(AppError::validation("Benchmark size must be between 1 byte and 4 GiB"));
        }
        if !(1..=100).contains(&self.repeat) {
            return Err(AppError::validation("--repeat must be between 1 and 100"));
        }
        if !(1..=256).contains(&self.threads) {
            return Err(AppError::validation("--threads must be between 1 and 256"));
        }
        if !(1024..=64 << 20).contains(&self.block_size) {
            return Err(AppError::validation("Block size must be between 1 KiB and 64 MiB"));
        }
        Ok(())
    }
}

/// Where the sample came from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SampleInfo {
    /// `generated` or the path read.
    pub source: String,
    pub bytes: u64,
    pub files: u64,
    pub seed: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompressionResult {
    /// `zstd-3`, `deflate-6`, `bzip2-9`, `stored`.
    pub name: String,
    pub codec: String,
    /// Engine level (`--compression-level`, 0–9).
    pub level: u32,
    /// The codec's own level (zstd 1–19, deflate/bzip2 1–9).
    pub native_level: i32,
    /// Usable with `--volume-format ufop` (zstd needs format 2) or only as
    /// a ZIP volume method.
    pub zip_only: bool,
    pub compressed_bytes: u64,
    /// compressed / original (lower is better).
    pub ratio: f64,
    pub compress_mb_per_sec: f64,
    pub decompress_mb_per_sec: f64,
    /// 1 = best balance of size and speed (see [`BenchReport::recommended`]).
    pub rank: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CipherResult {
    pub cipher: CipherKind,
    pub seal_mb_per_sec: f64,
    pub open_mb_per_sec: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KdfResult {
    pub name: String,
    pub params: KdfParams,
    pub millis: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChunkerResult {
    pub name: String,
    pub spec: ChunkerSpec,
    pub mb_per_sec: f64,
    pub chunks: u64,
    pub min_size: u64,
    pub max_size: u64,
    pub avg_size: f64,
    pub stddev: f64,
    pub p10: u64,
    pub p50: u64,
    pub p90: u64,
    /// Share of a modified copy of the sample (bytes inserted, deleted and
    /// overwritten in a few places) found in blocks of the original.
    pub dedup_ratio: f64,
    /// Bytes of the modified copy that would be uploaded again.
    pub new_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HashResult {
    pub algorithm: HashAlgorithm,
    pub mb_per_sec: f64,
}

/// Settings suggested by the measurements.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Recommendation {
    pub block_codec: Option<BlockCodec>,
    pub compression_level: Option<u32>,
    pub chunker: Option<ChunkerSpec>,
    pub block_size: Option<usize>,
    pub cipher: Option<CipherKind>,
    pub kdf: Option<KdfParams>,
    /// Why each choice was made.
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchReport {
    pub sample: SampleInfo,
    pub repeat: u32,
    pub threads: usize,
    pub block_size: usize,
    /// Ranked best first.
    pub compression: Vec<CompressionResult>,
    /// Fastest first.
    pub ciphers: Vec<CipherResult>,
    pub kdf: Vec<KdfResult>,
    /// Argon2id parameters measured to take about `kdf_target_ms`.
    pub kdf_suggestion: Option<KdfResult>,
    pub kdf_target_ms: u64,
    /// Best deduplication first.
    pub chunkers: Vec<ChunkerResult>,
    /// Fastest first.
    pub hashes: Vec<HashResult>,
    pub recommended: Recommendation,
    pub warnings: Vec<String>,
}

impl BenchReport {
    /// `ufop backup run` flags for a new repository with the recommended
    /// settings (Kopia's "usable initialization options").
    pub fn backup_run_flags(&self) -> Vec<String> {
        let r = &self.recommended;
        let mut f: Vec<String> = Vec::new();
        let mut add = |k: &str, v: String| {
            f.push(k.into());
            f.push(v);
        };
        if let Some(c) = r.block_codec {
            add("--block-codec", match c { BlockCodec::Deflate => "deflate", BlockCodec::Zstd => "zstd" }.into());
        }
        if let Some(l) = r.compression_level {
            add("--compression-level", l.to_string());
        }
        if let Some(c) = r.chunker {
            add("--chunker", if c.is_content_defined() { "fastcdc" } else { "fixed" }.into());
        }
        if let Some(b) = r.block_size {
            add("--block-size", b.to_string());
        }
        if let Some(c) = r.cipher {
            add("--cipher", c.as_str().replace('_', "-"));
        }
        if let Some(k) = r.kdf {
            add("--kdf-memory-kib", k.memory_kib.to_string());
            add("--kdf-iterations", k.iterations.to_string());
            add("--kdf-parallelism", k.parallelism.to_string());
        }
        f
    }

    /// [`backup_run_flags`](Self::backup_run_flags) as one shell line.
    pub fn backup_run_command(&self) -> String {
        let mut s = String::from("ufop backup run");
        for a in self.backup_run_flags() {
            s.push(' ');
            s.push_str(&a);
        }
        s
    }
}

/// Run the benchmark.
pub fn run(opts: &BenchOptions, ctx: &super::OpContext) -> Result<BenchReport, AppError> {
    opts.validate()?;
    let (data, sample) = match &opts.sample_path {
        Some(p) => read_sample(p, opts.size)?,
        None => (generate(opts.size, opts.seed), SampleInfo { source: "generated".into(), bytes: opts.size as u64, files: 0, seed: Some(opts.seed) }),
    };
    if data.is_empty() {
        return Err(AppError::validation("The benchmark sample is empty"));
    }
    let mut report = BenchReport {
        sample,
        repeat: opts.repeat,
        threads: opts.threads,
        block_size: opts.block_size,
        compression: Vec::new(),
        ciphers: Vec::new(),
        kdf: Vec::new(),
        kdf_suggestion: None,
        kdf_target_ms: opts.kdf_target_ms,
        chunkers: Vec::new(),
        hashes: Vec::new(),
        recommended: Recommendation::default(),
        warnings: Vec::new(),
    };
    let blocks: Vec<&[u8]> = data.chunks(opts.block_size).collect();
    let check = || if ctx.is_cancelled() { Err(AppError::state("The benchmark was cancelled", "Run it again when ready.")) } else { Ok(()) };
    if opts.runs(BenchSection::Compression) {
        check()?;
        report.compression = bench_compression(&blocks, opts)?;
    }
    if opts.runs(BenchSection::Crypto) {
        check()?;
        report.ciphers = bench_ciphers(&blocks, opts)?;
    }
    if opts.runs(BenchSection::Kdf) {
        check()?;
        let (kdf, suggestion) = bench_kdf(opts)?;
        report.kdf = kdf;
        report.kdf_suggestion = suggestion;
    }
    if opts.runs(BenchSection::Chunkers) {
        check()?;
        report.chunkers = bench_chunkers(&data, opts)?;
    }
    if opts.runs(BenchSection::Hashes) {
        check()?;
        report.hashes = bench_hashes(&blocks, opts);
    }
    if data.len() < 1 << 20 {
        report.warnings.push("The sample is smaller than 1 MiB; throughput figures are rough.".into());
    }
    report.recommended = recommend(&report);
    Ok(report)
}

/// Megabytes (10^6) per second for `bytes` in `d`.
fn mbps(bytes: usize, d: Duration) -> f64 {
    let s = d.as_secs_f64().max(1e-9);
    (bytes as f64 / 1e6) / s
}

/// Fastest of `repeat` runs of `f`.
fn best_of<T>(repeat: u32, mut f: impl FnMut() -> Result<T, AppError>) -> Result<(T, Duration), AppError> {
    let mut best: Option<(T, Duration)> = None;
    for _ in 0..repeat.max(1) {
        let t = Instant::now();
        let v = f()?;
        let d = t.elapsed();
        if best.as_ref().map_or(true, |(_, b)| d < *b) {
            best = Some((v, d));
        }
    }
    best.ok_or_else(|| AppError::internal("benchmark did not run"))
}

/// Apply `f` to every block on `threads` workers; results in block order.
fn parallel<T: Send>(blocks: &[&[u8]], threads: usize, f: impl Fn(usize, &[u8]) -> Result<T, AppError> + Sync) -> Result<Vec<T>, AppError> {
    let threads = threads.clamp(1, blocks.len().max(1));
    if threads == 1 {
        return blocks.iter().enumerate().map(|(i, b)| f(i, b)).collect();
    }
    let mut parts: Vec<Vec<(usize, T)>> = Vec::new();
    std::thread::scope(|s| -> Result<(), AppError> {
        let handles: Vec<_> = (0..threads)
            .map(|t| {
                let f = &f;
                s.spawn(move || -> Result<Vec<(usize, T)>, AppError> {
                    let mut out = Vec::new();
                    let mut i = t;
                    while i < blocks.len() {
                        out.push((i, f(i, blocks[i])?));
                        i += threads;
                    }
                    Ok(out)
                })
            })
            .collect();
        for h in handles {
            parts.push(h.join().map_err(|_| AppError::internal("benchmark worker panicked"))??);
        }
        Ok(())
    })?;
    let mut all: Vec<(usize, T)> = parts.into_iter().flatten().collect();
    all.sort_by_key(|(i, _)| *i);
    Ok(all.into_iter().map(|(_, v)| v).collect())
}

/// A codec the engine can write.
#[derive(Clone, Copy)]
enum Codec {
    Stored,
    Block(BlockCodec, u32),
    Bzip2(u32),
}

impl Codec {
    fn describe(self) -> (String, String, u32, i32, bool) {
        match self {
            Self::Stored => ("stored".into(), "stored".into(), 0, 0, false),
            Self::Block(BlockCodec::Deflate, l) => (format!("deflate-{l}"), "deflate".into(), l, l as i32, false),
            Self::Block(BlockCodec::Zstd, l) => (format!("zstd-{}", zstd_level(l)), "zstd".into(), l, zstd_level(l), false),
            Self::Bzip2(l) => (format!("bzip2-{l}"), "bzip2".into(), l, l as i32, true),
        }
    }

    /// (stored form, flag) of one block, exactly as a volume would hold it.
    fn encode(self, b: &[u8]) -> Result<(u8, Vec<u8>), AppError> {
        match self {
            Self::Stored => encode_block(VolumeFormat::Ufop, BlockCodec::Deflate, b, 0),
            Self::Block(c, l) => encode_block(VolumeFormat::Ufop, c, b, l),
            Self::Bzip2(l) => Ok((255, Bzip2.compress(b, l)?)),
        }
    }

    fn decode(flag: u8, d: &[u8]) -> Result<Vec<u8>, AppError> {
        if flag == 255 {
            Bzip2.decompress(d)
        } else {
            decode_block(flag, d)
        }
    }
}

fn bench_compression(blocks: &[&[u8]], opts: &BenchOptions) -> Result<Vec<CompressionResult>, AppError> {
    let total: usize = blocks.iter().map(|b| b.len()).sum();
    let mut codecs = vec![Codec::Stored];
    codecs.extend([1, 6, 9].map(|l| Codec::Block(BlockCodec::Deflate, l)));
    codecs.extend([1, 5, 7, 8, 9].map(|l| Codec::Block(BlockCodec::Zstd, l)));
    codecs.push(Codec::Bzip2(9));
    let mut out = Vec::new();
    for c in codecs {
        let (encoded, ct) = best_of(opts.repeat, || parallel(blocks, opts.threads, |_, b| c.encode(b)))?;
        let (decoded, dt) = best_of(opts.repeat, || parallel(&encoded.iter().map(|(_, d)| d.as_slice()).collect::<Vec<_>>(), opts.threads, |i, d| Codec::decode(encoded[i].0, d).map(|v| v.len())))?;
        if decoded.iter().sum::<usize>() != total {
            return Err(AppError::internal(format!("{} round trip lost data", c.describe().0)));
        }
        let compressed: usize = encoded.iter().map(|(_, d)| d.len()).sum();
        let (name, codec, level, native_level, zip_only) = c.describe();
        out.push(CompressionResult {
            name,
            codec,
            level,
            native_level,
            zip_only,
            compressed_bytes: compressed as u64,
            ratio: compressed as f64 / total.max(1) as f64,
            compress_mb_per_sec: mbps(total, ct),
            decompress_mb_per_sec: mbps(total, dt),
            rank: 0,
        });
    }
    rank_compression(&mut out);
    Ok(out)
}

/// Rank by a balance of size and speed: the bytes saved per second of
/// compression time matter, so a codec 1% smaller but 10× slower ranks
/// lower. Score = space saved × log10(compress MB/s + 1); stored ranks last.
fn rank_compression(v: &mut [CompressionResult]) {
    let score = |r: &CompressionResult| if r.codec == "stored" { f64::MIN } else { (1.0 - r.ratio) * (r.compress_mb_per_sec + 1.0).log10() };
    v.sort_by(|a, b| score(b).total_cmp(&score(a)).then(a.ratio.total_cmp(&b.ratio)));
    for (i, r) in v.iter_mut().enumerate() {
        r.rank = i as u32 + 1;
    }
}

fn bench_ciphers(blocks: &[&[u8]], opts: &BenchOptions) -> Result<Vec<CipherResult>, AppError> {
    let total: usize = blocks.iter().map(|b| b.len()).sum();
    let mut out = Vec::new();
    for kind in [CipherKind::Aes256Gcm, CipherKind::ChaCha20Poly1305] {
        let c = VolumeCipher::random_master(kind);
        let (sealed, st) = best_of(opts.repeat, || parallel(blocks, opts.threads, |_, b| c.seal(b)))?;
        let refs: Vec<&[u8]> = sealed.iter().map(|v| v.as_slice()).collect();
        let (opened, ot) = best_of(opts.repeat, || parallel(&refs, opts.threads, |_, b| c.open(b).map(|v| v.len())))?;
        if opened.iter().sum::<usize>() != total {
            return Err(AppError::internal("cipher round trip lost data"));
        }
        out.push(CipherResult { cipher: kind, seal_mb_per_sec: mbps(total, st), open_mb_per_sec: mbps(total, ot) });
    }
    out.sort_by(|a, b| (b.seal_mb_per_sec + b.open_mb_per_sec).total_cmp(&(a.seal_mb_per_sec + a.open_mb_per_sec)));
    Ok(out)
}

fn time_kdf(p: KdfParams) -> Result<f64, AppError> {
    let t = Instant::now();
    VolumeCipher::derive("ufop-benchmark-passphrase", b"ufop-benchmark-salt-0123", p)?;
    Ok(t.elapsed().as_secs_f64() * 1000.0)
}

fn bench_kdf(opts: &BenchOptions) -> Result<(Vec<KdfResult>, Option<KdfResult>), AppError> {
    let names: Vec<String> = if opts.kdf_presets.is_empty() { ["interactive", "default", "sensitive"].map(String::from).to_vec() } else { opts.kdf_presets.clone() };
    let mut out = Vec::new();
    for n in names {
        let params = KdfParams::preset(&n).ok_or_else(|| AppError::validation(format!("Unknown Argon2id preset '{n}' (interactive, default, sensitive)")))?;
        let (ms, _) = best_of(opts.repeat.min(3), || time_kdf(params))?;
        out.push(KdfResult { name: n, params, millis: ms });
    }
    let suggestion = if opts.kdf_target_ms > 0 { Some(suggest_kdf(opts)?) } else { None };
    Ok((out, suggestion))
}

/// Argon2id parameters taking about `kdf_target_ms`: time one pass at the
/// requested memory, scale the iterations (1–50), and halve the memory
/// (down to 19 MiB, OWASP's minimum) while one pass is already too slow.
fn suggest_kdf(opts: &BenchOptions) -> Result<KdfResult, AppError> {
    let target = opts.kdf_target_ms as f64;
    let parallelism = opts.kdf_parallelism.clamp(1, 64);
    let mut memory = opts.kdf_memory_kib.clamp(8 * 1024, 4 * 1024 * 1024).max(8 * parallelism);
    let floor = (19 * 1024).min(memory);
    let mut one = time_kdf(KdfParams { memory_kib: memory, iterations: 1, parallelism })?;
    while one > target && memory / 2 >= floor {
        memory /= 2;
        one = time_kdf(KdfParams { memory_kib: memory, iterations: 1, parallelism })?;
    }
    let iterations = ((target / one.max(0.01)).round() as u32).clamp(1, 50);
    let params = KdfParams { memory_kib: memory, iterations, parallelism };
    params.validate()?;
    let ms = time_kdf(params)?;
    Ok(KdfResult { name: "suggested".into(), params, millis: ms })
}

/// The chunker variants compared around `block_size`.
pub fn chunker_variants(block_size: usize) -> Vec<(String, ChunkerSpec)> {
    let b = block_size.max(1024);
    let kib = |n: usize| if n % 1024 == 0 { format!("{}K", n / 1024) } else { n.to_string() };
    vec![
        (format!("fixed-{}", kib(b)), ChunkerSpec::Fixed { size: b }),
        (format!("fastcdc-{}", kib(b / 2)), ChunkerSpec::fastcdc(b / 2)),
        (format!("fastcdc-{}", kib(b)), ChunkerSpec::fastcdc(b)),
        (format!("fastcdc-{}", kib(b * 2)), ChunkerSpec::fastcdc(b * 2)),
        (format!("fastcdc-{}-narrow", kib(b)), ChunkerSpec::FastCdc { min: b / 2, avg: b, max: b * 2 }),
    ]
}

fn chunk_sizes_and_hashes(data: &[u8], spec: ChunkerSpec) -> Result<Vec<(usize, String)>, AppError> {
    let c = spec.build();
    let mut out = Vec::new();
    for_each_chunk(&mut &data[..], c.as_ref(), |b| {
        out.push((b.len(), hash_bytes(HashAlgorithm::Sha256, b)));
        Ok(true)
    })
    .map_err(AppError::internal)?;
    Ok(out)
}

/// A copy of `data` with a few edits a user might make: bytes inserted
/// near the start, a run overwritten in the middle and bytes deleted near
/// the end.
pub fn modified_copy(data: &[u8], seed: u64) -> Vec<u8> {
    let mut rng = SplitMix64(seed ^ 0x6d6f_6469_6679);
    let n = data.len();
    let mut out = Vec::with_capacity(n + 64);
    let ins = n / 7;
    out.extend_from_slice(&data[..ins]);
    out.extend((0..37).map(|_| rng.next() as u8));
    out.extend_from_slice(&data[ins..]);
    let mid = out.len() / 2;
    let end = (mid + 16).min(out.len());
    for b in &mut out[mid..end] {
        *b = rng.next() as u8;
    }
    let del = out.len() * 5 / 6;
    let del_end = (del + 53).min(out.len());
    out.drain(del..del_end);
    out
}

fn bench_chunkers(data: &[u8], opts: &BenchOptions) -> Result<Vec<ChunkerResult>, AppError> {
    let modified = modified_copy(data, opts.seed);
    let mut out = Vec::new();
    for (name, spec) in chunker_variants(opts.block_size) {
        let (chunks, t) = best_of(opts.repeat, || chunk_sizes_and_hashes(data, spec))?;
        let known: HashSet<&str> = chunks.iter().map(|(_, h)| h.as_str()).collect();
        let changed = chunk_sizes_and_hashes(&modified, spec)?;
        let new_bytes: usize = changed.iter().filter(|(_, h)| !known.contains(h.as_str())).map(|(n, _)| *n).sum();
        let mut sizes: Vec<u64> = chunks.iter().map(|(n, _)| *n as u64).collect();
        sizes.sort_unstable();
        let count = sizes.len().max(1) as f64;
        let avg = sizes.iter().sum::<u64>() as f64 / count;
        let var = sizes.iter().map(|s| (*s as f64 - avg).powi(2)).sum::<f64>() / count;
        let pct = |p: usize| sizes.get((sizes.len().saturating_sub(1)) * p / 100).copied().unwrap_or(0);
        out.push(ChunkerResult {
            name,
            spec,
            mb_per_sec: mbps(data.len(), t),
            chunks: sizes.len() as u64,
            min_size: sizes.first().copied().unwrap_or(0),
            max_size: sizes.last().copied().unwrap_or(0),
            avg_size: avg,
            stddev: var.sqrt(),
            p10: pct(10),
            p50: pct(50),
            p90: pct(90),
            dedup_ratio: 1.0 - new_bytes as f64 / modified.len().max(1) as f64,
            new_bytes: new_bytes as u64,
        });
    }
    out.sort_by(|a, b| b.dedup_ratio.total_cmp(&a.dedup_ratio).then(b.mb_per_sec.total_cmp(&a.mb_per_sec)));
    Ok(out)
}

fn bench_hashes(blocks: &[&[u8]], opts: &BenchOptions) -> Vec<HashResult> {
    let total: usize = blocks.iter().map(|b| b.len()).sum();
    let mut out: Vec<HashResult> = [HashAlgorithm::Sha256, HashAlgorithm::Sha512]
        .into_iter()
        .filter_map(|alg| best_of(opts.repeat, || parallel(blocks, opts.threads, |_, b| Ok(hash_bytes(alg, b)))).ok().map(|(_, t)| HashResult { algorithm: alg, mb_per_sec: mbps(total, t) }))
        .collect();
    out.sort_by(|a, b| b.mb_per_sec.total_cmp(&a.mb_per_sec));
    out
}

/// Pick settings from the measurements.
pub fn recommend(r: &BenchReport) -> Recommendation {
    let mut rec = Recommendation::default();
    // Compression: the best-ranked codec a format 2 UFOP repository can
    // write; incompressible data (< 3% saved) is stored.
    if let Some(best) = r.compression.iter().find(|c| !c.zip_only && c.codec != "stored") {
        if best.ratio > 0.97 {
            rec.compression_level = Some(0);
            rec.reasons.push(format!("The sample barely compresses ({:.0}% of its size at best); store blocks uncompressed.", best.ratio * 100.0));
        } else {
            rec.block_codec = Some(if best.codec == "zstd" { BlockCodec::Zstd } else { BlockCodec::Deflate });
            rec.compression_level = Some(best.level);
            rec.reasons.push(format!("{} keeps {:.0}% of the size at {:.0} MB/s.", best.name, best.ratio * 100.0, best.compress_mb_per_sec));
        }
    }
    if let Some(c) = r.ciphers.first() {
        rec.cipher = Some(c.cipher);
        rec.reasons.push(format!("{} is the faster cipher here ({:.0} MB/s sealing).", c.cipher.as_str(), c.seal_mb_per_sec));
    }
    if let Some(k) = &r.kdf_suggestion {
        rec.kdf = Some(k.params);
        rec.reasons.push(format!(
            "Argon2id with {} MiB × {} iterations takes about {:.0} ms (target {} ms).",
            k.params.memory_kib / 1024,
            k.params.iterations,
            k.millis,
            r.kdf_target_ms
        ));
    }
    // Chunker: the best-deduplicating FastCDC variant expressible as
    // `--chunker fastcdc --block-size N` (min = N/4, max = 4N).
    if let Some(c) = r.chunkers.iter().find(|c| matches!(c.spec, ChunkerSpec::FastCdc { min, avg, max } if min == avg / 4 && max == avg * 4)) {
        if let ChunkerSpec::FastCdc { avg, .. } = c.spec {
            rec.chunker = Some(c.spec);
            rec.block_size = Some(avg);
            rec.reasons.push(format!("{} re-uploads {:.1}% of an edited copy.", c.name, (1.0 - c.dedup_ratio) * 100.0));
        }
    }
    rec
}

/// SplitMix64: a small fixed generator so a seed always gives the same data.
struct SplitMix64(u64);

impl SplitMix64 {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

const WORDS: &[&str] = &[
    "the", "backup", "file", "of", "and", "version", "block", "data", "to", "restore", "folder", "a", "in", "is", "for", "repository", "with",
    "chunk", "on", "that", "report", "invoice", "meeting", "project", "photo", "2026", "summary", "budget", "draft", "final", "notes", "team",
];

/// `size` bytes of mixed sample data: ~50% prose, ~25% structured records
/// (CSV/JSON-like lines), ~25% random bytes (like already-compressed media).
pub fn generate(size: usize, seed: u64) -> Vec<u8> {
    let mut rng = SplitMix64(seed);
    let mut out = Vec::with_capacity(size + 4096);
    let mut record = 0u64;
    while out.len() < size {
        let seg = 1024 + rng.below(15 * 1024) as usize;
        let end = (out.len() + seg).min(size);
        match rng.below(4) {
            0 | 1 => {
                while out.len() < end {
                    out.extend_from_slice(WORDS[rng.below(WORDS.len() as u64) as usize].as_bytes());
                    out.push(if rng.below(12) == 0 { b'\n' } else { b' ' });
                }
            }
            2 => {
                while out.len() < end {
                    record += 1;
                    let line = format!("{{\"id\":{record},\"size\":{},\"name\":\"{}\",\"ok\":{}}}\n", rng.below(1 << 20), WORDS[rng.below(WORDS.len() as u64) as usize], rng.below(2) == 0);
                    out.extend_from_slice(line.as_bytes());
                }
            }
            _ => {
                while out.len() < end {
                    out.extend_from_slice(&rng.next().to_le_bytes());
                }
            }
        }
        out.truncate(end);
    }
    out
}

/// Read up to `limit` bytes from a file, or from the files of a folder in
/// name order (symbolic links are not followed).
pub fn read_sample(path: &Path, limit: usize) -> Result<(Vec<u8>, SampleInfo), AppError> {
    use std::io::Read;
    let md = std::fs::symlink_metadata(path).map_err(|e| AppError::file_op(format!("Cannot read the sample {}: {e}", path.display()), "Pass an existing file or folder."))?;
    let mut files = Vec::new();
    if md.is_dir() {
        let mut stack = vec![path.to_path_buf()];
        while let Some(d) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&d) else { continue };
            for e in rd.filter_map(|e| e.ok()) {
                match e.file_type() {
                    Ok(t) if t.is_dir() => stack.push(e.path()),
                    Ok(t) if t.is_file() => files.push(e.path()),
                    _ => {}
                }
            }
        }
        files.sort();
    } else {
        files.push(path.to_path_buf());
    }
    let mut data = Vec::new();
    let mut used = 0u64;
    for f in files {
        if data.len() >= limit {
            break;
        }
        let Ok(file) = std::fs::File::open(&f) else { continue };
        let before = data.len();
        file.take((limit - data.len()) as u64).read_to_end(&mut data).map_err(|e| AppError::file_op(format!("Cannot read {}: {e}", f.display()), "Check the file is readable."))?;
        used += (data.len() > before) as u64;
    }
    let bytes = data.len() as u64;
    Ok((data, SampleInfo { source: path.display().to_string(), bytes, files: used, seed: None }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::OpContext;

    fn small(sections: &[BenchSection]) -> BenchOptions {
        BenchOptions {
            size: 192 * 1024,
            repeat: 1,
            block_size: 16 * 1024,
            sections: sections.to_vec(),
            kdf_target_ms: 1,
            kdf_memory_kib: 8 * 1024,
            kdf_parallelism: 1,
            kdf_presets: vec!["interactive".into()],
            ..Default::default()
        }
    }

    #[test]
    fn generated_data_is_deterministic_and_compressible() {
        let a = generate(100_000, 7);
        assert_eq!(a.len(), 100_000);
        assert_eq!(a, generate(100_000, 7));
        assert_ne!(a, generate(100_000, 8));
        let (_, z) = encode_block(VolumeFormat::Ufop, BlockCodec::Zstd, &a, 6).unwrap();
        assert!(z.len() < a.len() * 9 / 10, "{} of {}", z.len(), a.len());
    }

    #[test]
    fn compression_section_ranks_every_codec_and_round_trips() {
        let r = run(&BenchOptions { threads: 2, ..small(&[BenchSection::Compression]) }, &OpContext::new()).unwrap();
        let names: Vec<&str> = r.compression.iter().map(|c| c.name.as_str()).collect();
        for n in ["stored", "deflate-1", "deflate-6", "deflate-9", "zstd-1", "zstd-3", "zstd-19", "bzip2-9"] {
            assert!(names.contains(&n), "{n} missing from {names:?}");
        }
        assert_eq!(r.compression.last().unwrap().codec, "stored");
        assert_eq!(r.compression.iter().map(|c| c.rank).collect::<Vec<_>>(), (1..=r.compression.len() as u32).collect::<Vec<_>>());
        let stored = r.compression.iter().find(|c| c.codec == "stored").unwrap();
        assert_eq!((stored.ratio, stored.compressed_bytes), (1.0, r.sample.bytes));
        assert!(r.compression.iter().all(|c| c.ratio <= 1.0 && c.compress_mb_per_sec > 0.0));
        // Sections not asked for stay empty, and no cipher / KDF is suggested.
        assert!(r.ciphers.is_empty() && r.kdf.is_empty() && r.chunkers.is_empty() && r.hashes.is_empty());
        assert!(r.recommended.block_codec.is_some() && r.recommended.cipher.is_none());
    }

    #[test]
    fn incompressible_samples_recommend_storing() {
        let dir = tempfile::tempdir().unwrap();
        let mut rng = SplitMix64(3);
        let random: Vec<u8> = (0..64 * 1024).map(|_| rng.next() as u8).collect();
        std::fs::write(dir.path().join("a.bin"), &random).unwrap();
        let r = run(&BenchOptions { sample_path: Some(dir.path().into()), ..small(&[BenchSection::Compression]) }, &OpContext::new()).unwrap();
        assert_eq!(r.sample.bytes, 64 * 1024);
        assert_eq!(r.recommended.compression_level, Some(0));
        assert_eq!(r.recommended.block_codec, None);
        assert_eq!(r.backup_run_flags(), vec!["--compression-level", "0"]);
    }

    #[test]
    fn crypto_kdf_and_hash_sections() {
        let r = run(&small(&[BenchSection::Crypto, BenchSection::Kdf, BenchSection::Hashes]), &OpContext::new()).unwrap();
        let ciphers: HashSet<CipherKind> = r.ciphers.iter().map(|c| c.cipher).collect();
        assert_eq!(ciphers, HashSet::from([CipherKind::Aes256Gcm, CipherKind::ChaCha20Poly1305]));
        assert_eq!(r.kdf.len(), 1);
        assert_eq!(r.kdf[0].params, KdfParams::preset("interactive").unwrap());
        let s = r.kdf_suggestion.clone().unwrap();
        // A 1 ms target cannot be met: one iteration at the smallest allowed memory.
        assert_eq!((s.params.iterations, s.params.memory_kib, s.params.parallelism), (1, 8 * 1024, 1));
        assert_eq!(r.hashes.len(), 2);
        let flags = r.backup_run_flags();
        assert!(flags.windows(2).any(|w| w[0] == "--cipher" && (w[1] == "aes256-gcm" || w[1] == "chacha20-poly1305")), "{flags:?}");
        assert!(flags.windows(2).any(|w| w == ["--kdf-iterations", "1"]));
        assert!(r.backup_run_command().starts_with("ufop backup run --cipher"));
    }

    #[test]
    fn chunkers_report_distribution_and_fastcdc_dedups_edits_better() {
        let r = run(&small(&[BenchSection::Chunkers]), &OpContext::new()).unwrap();
        assert_eq!(r.chunkers.len(), 5);
        let fixed = r.chunkers.iter().find(|c| c.name.starts_with("fixed")).unwrap();
        let cdc = r.chunkers.iter().find(|c| c.name == "fastcdc-16K").unwrap();
        assert!(cdc.dedup_ratio > fixed.dedup_ratio, "cdc {} fixed {}", cdc.dedup_ratio, fixed.dedup_ratio);
        assert_eq!(fixed.max_size, 16 * 1024);
        for c in &r.chunkers {
            assert!(c.chunks > 0 && c.min_size <= c.p10 && c.p10 <= c.p50 && c.p50 <= c.p90 && c.p90 <= c.max_size, "{c:?}");
            assert_eq!((c.avg_size * c.chunks as f64).round() as u64, r.sample.bytes);
        }
        let rec = &r.recommended;
        assert!(rec.chunker.is_some_and(|c| c.is_content_defined()));
        let flags = r.backup_run_flags();
        assert!(flags.windows(2).any(|w| w == ["--chunker", "fastcdc"]), "{flags:?}");
        assert!(flags.contains(&"--block-size".to_string()));
    }

    #[test]
    fn modified_copy_differs_slightly() {
        let a = generate(50_000, 1);
        let b = modified_copy(&a, 1);
        assert_eq!(b.len(), a.len() + 37 - 53);
        assert_eq!(&a[..1000], &b[..1000]);
    }

    #[test]
    fn options_are_validated_and_sections_parse() {
        assert!(run(&BenchOptions { repeat: 0, ..small(&[]) }, &OpContext::new()).is_err());
        assert!(run(&BenchOptions { block_size: 10, ..small(&[]) }, &OpContext::new()).is_err());
        assert!(run(&BenchOptions { kdf_presets: vec!["nope".into()], ..small(&[BenchSection::Kdf]) }, &OpContext::new()).is_err());
        assert_eq!(BenchSection::parse("Ciphers"), Some(BenchSection::Crypto));
        assert_eq!(BenchSection::parse("bogus"), None);
        let ctx = OpContext::new();
        ctx.cancel();
        assert!(run(&small(&[]), &ctx).is_err());
    }

    #[test]
    fn folder_samples_are_capped_and_read_in_name_order() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("b.txt"), b"bbbb").unwrap();
        std::fs::write(dir.path().join("a.txt"), b"aaaa").unwrap();
        std::fs::write(dir.path().join("sub/c.txt"), b"cccc").unwrap();
        let (d, info) = read_sample(dir.path(), 10).unwrap();
        assert_eq!(d, b"aaaabbbbcc");
        assert_eq!(info.files, 3);
        assert!(read_sample(&dir.path().join("missing"), 10).is_err());
    }
}
