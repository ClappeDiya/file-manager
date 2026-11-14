//! Splitting files into deduplication blocks.
//!
//! [`Chunker`] decides where blocks end; [`for_each_chunk`] streams a reader
//! through it. Two implementations exist:
//!
//! - [`FixedChunker`]: every block is `size` bytes except the last (format v1
//!   and an option for v2). Byte-identical to the original v1 reader.
//! - [`FastCdc`]: content-defined chunking (FastCDC, Xia et al. 2016, with
//!   normalized chunking). Block boundaries depend on the content, so an
//!   insertion near the start of a file shifts only the blocks around it
//!   and the rest still deduplicates. Default for new v2 repositories.
//!
//! The chunker of a repository is recorded in its header ([`ChunkerSpec`]):
//! changing it would make new blocks stop matching old ones.

use serde::{Deserialize, Serialize};
use std::io::Read;

/// Chunker recorded in the repository header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChunkerSpec {
    Fixed { size: usize },
    #[serde(rename = "fastcdc")]
    FastCdc { min: usize, avg: usize, max: usize },
}

impl ChunkerSpec {
    /// FastCDC around `avg` bytes: min = avg / 4, max = avg × 4.
    pub fn fastcdc(avg: usize) -> Self {
        let avg = avg.max(256);
        Self::FastCdc { min: avg / 4, avg, max: avg * 4 }
    }

    pub fn build(self) -> Box<dyn Chunker> {
        match self {
            Self::Fixed { size } => Box::new(FixedChunker { size: size.max(1) }),
            Self::FastCdc { min, avg, max } => Box::new(FastCdc::new(min, avg, max)),
        }
    }

    /// Largest block this chunker emits.
    pub fn max_size(self) -> usize {
        match self {
            Self::Fixed { size } => size.max(1),
            Self::FastCdc { max, .. } => max.max(1),
        }
    }

    pub fn is_content_defined(self) -> bool {
        matches!(self, Self::FastCdc { .. })
    }
}

/// Where the next block ends.
pub trait Chunker: Send + Sync {
    /// Largest block (the read buffer size).
    fn max_size(&self) -> usize;
    /// Length of the block at the start of `buf`. `buf` holds `max_size()`
    /// bytes unless the input ends (`eof`); the result is in `1..=buf.len()`.
    fn cut(&self, buf: &[u8], eof: bool) -> usize;
}

pub struct FixedChunker {
    pub size: usize,
}

impl Chunker for FixedChunker {
    fn max_size(&self) -> usize {
        self.size
    }
    fn cut(&self, buf: &[u8], _eof: bool) -> usize {
        buf.len().min(self.size)
    }
}

/// FastCDC with normalized chunking (level 1): a stricter mask before the
/// average size and a looser one after it keeps block sizes near `avg`.
pub struct FastCdc {
    min: usize,
    avg: usize,
    max: usize,
    mask_s: u64,
    mask_l: u64,
}

/// The 256-entry Gear table. It is part of the repository format, so it is
/// generated from a fixed seed (SplitMix64) and must never change.
fn gear() -> &'static [u64; 256] {
    static TABLE: std::sync::OnceLock<[u64; 256]> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t = [0u64; 256];
        let mut x: u64 = 0x5546_4f50_4344_4331; // "UFOPCDC1"
        for v in t.iter_mut() {
            x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = x;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            *v = z ^ (z >> 31);
        }
        t
    })
}

/// A mask of `bits` one-bits in the high end of the word (the Gear hash
/// shifts left, so high bits depend on the most recent ~64 bytes).
fn high_mask(bits: u32) -> u64 {
    let bits = bits.clamp(1, 63);
    !0u64 << (64 - bits)
}

impl FastCdc {
    pub fn new(min: usize, avg: usize, max: usize) -> Self {
        let avg = avg.max(64);
        let min = min.clamp(1, avg);
        let max = max.max(avg);
        let bits = (avg as f64).log2().round() as u32;
        Self { min, avg, max, mask_s: high_mask(bits + 1), mask_l: high_mask(bits.saturating_sub(1)) }
    }
}

impl Chunker for FastCdc {
    fn max_size(&self) -> usize {
        self.max
    }

    fn cut(&self, buf: &[u8], _eof: bool) -> usize {
        let n = buf.len();
        if n <= self.min {
            return n;
        }
        let g = gear();
        let center = self.avg.min(n);
        let end = self.max.min(n);
        let mut h = 0u64;
        let mut i = self.min;
        while i < center {
            h = (h << 1).wrapping_add(g[buf[i] as usize]);
            if h & self.mask_s == 0 {
                return i + 1;
            }
            i += 1;
        }
        while i < end {
            h = (h << 1).wrapping_add(g[buf[i] as usize]);
            if h & self.mask_l == 0 {
                return i + 1;
            }
            i += 1;
        }
        end
    }
}

/// Stream `r` through `chunker`, calling `f` for every block in order.
/// `f` returns `false` to stop early. Returns the bytes consumed.
pub fn for_each_chunk(
    r: &mut dyn Read,
    chunker: &dyn Chunker,
    mut f: impl FnMut(&[u8]) -> Result<bool, String>,
) -> Result<u64, String> {
    let max = chunker.max_size().max(1);
    let mut buf = vec![0u8; max];
    let mut filled = 0usize;
    let mut eof = false;
    let mut total = 0u64;
    loop {
        while !eof && filled < max {
            match r.read(&mut buf[filled..]) {
                Ok(0) => eof = true,
                Ok(n) => filled += n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e.to_string()),
            }
        }
        if filled == 0 {
            return Ok(total);
        }
        let n = chunker.cut(&buf[..filled], eof).clamp(1, filled);
        total += n as u64;
        if !f(&buf[..n])? {
            return Ok(total);
        }
        buf.copy_within(n..filled, 0);
        filled -= n;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::{hash_bytes, HashAlgorithm};
    use std::collections::HashSet;

    fn pseudo_random(len: usize, seed: u64) -> Vec<u8> {
        let mut x = seed;
        (0..len)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                (x >> 24) as u8
            })
            .collect()
    }

    fn chunks(data: &[u8], spec: ChunkerSpec) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        let c = spec.build();
        for_each_chunk(&mut std::io::Cursor::new(data), c.as_ref(), |b| {
            out.push(b.to_vec());
            Ok(true)
        })
        .unwrap();
        out
    }

    #[test]
    fn fixed_matches_plain_block_split() {
        let data = pseudo_random(10_000, 1);
        let c = chunks(&data, ChunkerSpec::Fixed { size: 1024 });
        let want: Vec<Vec<u8>> = data.chunks(1024).map(<[u8]>::to_vec).collect();
        assert_eq!(c, want);
        assert!(chunks(&[], ChunkerSpec::Fixed { size: 1024 }).is_empty());
        // Exact multiple: no empty trailing block.
        assert_eq!(chunks(&data[..2048], ChunkerSpec::Fixed { size: 1024 }).len(), 2);
    }

    #[test]
    fn fastcdc_bounds_and_reassembly() {
        let data = pseudo_random(1 << 20, 7);
        let spec = ChunkerSpec::fastcdc(8192);
        let c = chunks(&data, spec);
        assert_eq!(c.concat(), data);
        let (min, max) = match spec {
            ChunkerSpec::FastCdc { min, max, .. } => (min, max),
            _ => unreachable!(),
        };
        for b in &c[..c.len() - 1] {
            assert!(b.len() >= min && b.len() <= max, "{}", b.len());
        }
        let avg = data.len() / c.len();
        assert!((4096..=16384).contains(&avg), "average block {avg}");
        // Deterministic.
        assert_eq!(chunks(&data, spec), c);
    }

    /// An insertion near the start of a large file keeps most blocks under
    /// content-defined chunking but shifts every block under fixed chunking.
    #[test]
    fn insertion_reuses_blocks_only_with_cdc() {
        let data = pseudo_random(2 << 20, 42);
        let mut edited = data[..1000].to_vec();
        edited.extend_from_slice(b"inserted bytes!");
        edited.extend_from_slice(&data[1000..]);
        let reuse = |spec: ChunkerSpec| {
            let before: HashSet<String> = chunks(&data, spec).iter().map(|b| hash_bytes(HashAlgorithm::Sha256, b)).collect();
            let after = chunks(&edited, spec);
            let same = after.iter().filter(|b| before.contains(&hash_bytes(HashAlgorithm::Sha256, b))).count();
            same as f64 / after.len() as f64
        };
        let cdc = reuse(ChunkerSpec::fastcdc(16 * 1024));
        let fixed = reuse(ChunkerSpec::Fixed { size: 16 * 1024 });
        assert!(cdc > 0.9, "CDC reused {cdc}");
        assert!(fixed < 0.05, "fixed reused {fixed}");
    }

    #[test]
    fn spec_serde_shape() {
        let s = serde_json::to_value(ChunkerSpec::fastcdc(4096)).unwrap();
        assert_eq!(s, serde_json::json!({"type": "fastcdc", "min": 1024, "avg": 4096, "max": 16384}));
        let f: ChunkerSpec = serde_json::from_value(serde_json::json!({"type": "fixed", "size": 10})).unwrap();
        assert_eq!(f, ChunkerSpec::Fixed { size: 10 });
    }
}
