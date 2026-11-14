//! Help from the app's own documentation, offline.
//!
//! The user-facing guides in `docs/` are compiled into the app, split into
//! sections at their headings, and searched with BM25 (section titles
//! count double). The assistant answers questions from the best sections:
//! with a local model it writes an answer grounded in them, without one it
//! shows them directly. Either way the sources are listed.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::OnceLock;

/// (title, markdown) of each bundled guide.
const DOCS: &[(&str, &str)] = &[
    ("User Guide", include_str!("../../../docs/user-guide.md")),
    ("Troubleshooting", include_str!("../../../docs/troubleshooting.md")),
    ("CLI Reference", include_str!("../../../docs/cli-reference.md")),
    ("Backup destinations", include_str!("../../../docs/backup-destinations.md")),
    ("Backup encryption and secrets", include_str!("../../../docs/backup-encryption-and-secrets.md")),
    ("Backup logging and support", include_str!("../../../docs/backup-logging-and-support.md")),
    ("Silent install", include_str!("../../../docs/silent-install.md")),
    ("Migration guide", include_str!("../../../docs/migration-guide.md")),
];

const STOP: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "by", "can", "do", "does", "for", "from", "how", "i", "if", "in", "is", "it", "its", "me", "my", "of", "on", "or", "so", "that", "the", "this", "to", "what", "when", "where", "which", "why", "with", "you", "your", "will", "would", "should", "could", "there", "get", "use",
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DocHit {
    /// Guide title.
    pub doc: String,
    /// Section heading path (`Sync › Conflicts`).
    pub section: String,
    /// The section's text (markdown), trimmed.
    pub excerpt: String,
    pub score: f64,
}

struct Chunk {
    doc: &'static str,
    section: String,
    text: String,
    tf: HashMap<String, f64>,
    len: f64,
}

pub struct DocIndex {
    chunks: Vec<Chunk>,
    df: HashMap<String, f64>,
    avg_len: f64,
}

fn tokens(s: &str) -> impl Iterator<Item = String> + '_ {
    s.split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-')
        .map(|w| w.trim_matches('-').to_lowercase())
        .filter(|w| w.chars().count() >= 2 && !STOP.contains(&w.as_str()))
        .map(|w| stem(&w))
}

/// Light stemming so "conflicts" finds "conflict" and "syncing" finds "sync".
fn stem(w: &str) -> String {
    for suffix in ["ing", "es", "ed", "s"] {
        if let Some(base) = w.strip_suffix(suffix) {
            if base.chars().count() >= 3 && !(suffix == "s" && base.ends_with('s')) {
                return base.to_string();
            }
        }
    }
    w.to_string()
}

impl DocIndex {
    /// Index markdown documents, one chunk per `##`/`###` section.
    pub fn build(docs: &[(&'static str, &str)]) -> Self {
        let mut chunks = Vec::new();
        for (doc, md) in docs {
            let (mut h2, mut h3, mut body) = (String::new(), String::new(), String::new());
            let mut in_code = false;
            let mut flush = |h2: &str, h3: &str, body: &mut String| {
                let text = body.trim().to_string();
                body.clear();
                if text.len() < 40 || h2.eq_ignore_ascii_case("table of contents") {
                    return;
                }
                let section = if h3.is_empty() { h2.to_string() } else { format!("{h2} › {h3}") };
                let mut tf: HashMap<String, f64> = HashMap::new();
                for t in tokens(&text) {
                    *tf.entry(t).or_default() += 1.0;
                }
                for t in tokens(&section) {
                    *tf.entry(t).or_default() += 2.0;
                }
                let len = tf.values().sum();
                chunks.push(Chunk { doc, section, text, tf, len });
            };
            for line in md.lines() {
                if line.trim_start().starts_with("```") {
                    in_code = !in_code;
                }
                if !in_code && line.starts_with("## ") {
                    flush(&h2, &h3, &mut body);
                    (h2, h3) = (line[3..].trim().to_string(), String::new());
                } else if !in_code && line.starts_with("### ") {
                    flush(&h2, &h3, &mut body);
                    h3 = line[4..].trim().to_string();
                } else if !in_code && line.starts_with("# ") {
                    continue;
                } else {
                    body.push_str(line);
                    body.push('\n');
                }
            }
            flush(&h2, &h3, &mut body);
        }
        let mut df: HashMap<String, f64> = HashMap::new();
        for c in &chunks {
            for t in c.tf.keys() {
                *df.entry(t.clone()).or_default() += 1.0;
            }
        }
        let avg_len = chunks.iter().map(|c| c.len).sum::<f64>() / chunks.len().max(1) as f64;
        DocIndex { chunks, df, avg_len }
    }

    /// The bundled guides (built once).
    pub fn bundled() -> &'static DocIndex {
        static INDEX: OnceLock<DocIndex> = OnceLock::new();
        INDEX.get_or_init(|| DocIndex::build(DOCS))
    }

    pub fn len(&self) -> usize {
        self.chunks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }

    /// The `k` best sections for `query` (BM25), best first; none when nothing matches.
    pub fn search(&self, query: &str, k: usize) -> Vec<DocHit> {
        let terms: Vec<String> = {
            let mut t: Vec<String> = tokens(query).collect();
            t.sort();
            t.dedup();
            t
        };
        if terms.is_empty() {
            return Vec::new();
        }
        let n = self.chunks.len() as f64;
        let (k1, b) = (1.2, 0.75);
        let mut scored: Vec<(f64, usize)> = self
            .chunks
            .iter()
            .enumerate()
            .filter_map(|(i, c)| {
                let mut score = 0.0;
                for t in &terms {
                    let Some(&f) = c.tf.get(t) else { continue };
                    let df = self.df.get(t).copied().unwrap_or(0.0);
                    let idf = ((n - df + 0.5) / (df + 0.5) + 1.0).ln();
                    score += idf * f * (k1 + 1.0) / (f + k1 * (1.0 - b + b * c.len / self.avg_len));
                }
                (score > 0.0).then_some((score, i))
            })
            .collect();
        scored.sort_by(|a, b| b.0.total_cmp(&a.0));
        scored
            .into_iter()
            .take(k)
            .map(|(score, i)| {
                let c = &self.chunks[i];
                DocHit { doc: c.doc.to_string(), section: c.section.clone(), excerpt: excerpt(&c.text, 1200), score }
            })
            .collect()
    }
}

fn excerpt(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max).collect();
    // End at a paragraph or sentence when one is near.
    let end = cut.rfind("\n\n").or_else(|| cut.rfind(". ").map(|i| i + 1)).filter(|&i| i > max / 2).unwrap_or(cut.len());
    format!("{}…", cut[..end].trim_end())
}

/// The prompt that asks a model to answer from `hits` only.
pub fn grounded_prompt(question: &str, hits: &[DocHit]) -> String {
    let mut p = String::from(
        "Answer the user's question about the UFOP file manager using ONLY the documentation excerpts below. \
         Be brief and practical (steps or a short paragraph). If the excerpts don't answer it, say so and suggest the closest section. \
         Do not invent menu items or options.\n\n",
    );
    for (i, h) in hits.iter().enumerate() {
        p.push_str(&format!("[{}] {} — {}\n{}\n\n", i + 1, h.doc, h.section, h.excerpt));
    }
    p.push_str(&format!("Question: {question}\nAnswer:"));
    p
}

/// What to show without a model: the best section, and where the others are.
pub fn offline_answer(hits: &[DocHit]) -> String {
    let Some(first) = hits.first() else {
        return "I couldn't find that in the documentation. Try other words, or open the User Guide.".into();
    };
    let mut s = format!("From the {} — **{}**:\n\n{}", first.doc, first.section, first.excerpt);
    if hits.len() > 1 {
        s.push_str("\n\nSee also: ");
        s.push_str(&hits[1..].iter().map(|h| format!("{} — {}", h.doc, h.section)).collect::<Vec<_>>().join("; "));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "# Guide\n\n## Table of Contents\n1. Sync\n\n## Sync\n\nKeep two folders the same. Choose one-way or two-way and a schedule for the sync pair.\n\n### Conflicts\n\nWhen both sides changed, the conflict is kept as a copy named file (conflict).txt; open the Conflicts panel to choose.\n\n## Transfers\n\nTransfers resume after a crash using the journal. Checksums verify every chunk.\n\n```\n## not a heading inside code\n```\n";

    #[test]
    fn sections_rank_by_relevance_with_titles_weighted() {
        let idx = DocIndex::build(&[("Guide", SAMPLE)]);
        assert_eq!(idx.len(), 3, "table of contents skipped, code fences not split");
        let hits = idx.search("how are sync conflicts resolved?", 3);
        assert_eq!(hits[0].section, "Sync › Conflicts");
        assert!(hits[0].excerpt.contains("Conflicts panel"));
        assert_eq!(idx.search("resume transfer after crash", 1)[0].section, "Transfers");
        assert!(idx.search("the of and", 3).is_empty(), "stop words only");
        assert!(idx.search("kubernetes", 3).is_empty());
    }

    #[test]
    fn bundled_guides_answer_real_questions() {
        let idx = DocIndex::bundled();
        assert!(idx.len() > 50, "{}", idx.len());
        let top = |q: &str| idx.search(q, 3).into_iter().map(|h| format!("{} / {}", h.doc, h.section)).collect::<Vec<_>>();
        let has = |q: &str, want: &str| {
            let t = top(q);
            assert!(t.iter().any(|s| s.contains(want)), "'{q}' → {t:?}");
        };
        has("decrypt an old gpg file without integrity protection", "OpenPGP");
        has("validate a batch of files with a sha256 manifest", "validating a batch");
        has("convert documents to pdf", "Converting");
    }

    #[test]
    fn prompts_and_offline_answers_cite_sections() {
        let idx = DocIndex::build(&[("Guide", SAMPLE)]);
        let hits = idx.search("sync conflicts", 2);
        let p = grounded_prompt("What happens on a conflict?", &hits);
        assert!(p.contains("[1] Guide — Sync › Conflicts") && p.ends_with("Answer:"));
        let a = offline_answer(&hits);
        assert!(a.starts_with("From the Guide — **Sync › Conflicts**") && a.contains("See also: Guide — Sync"));
        assert!(offline_answer(&[]).contains("couldn't find"));
        assert!(excerpt(&"word. ".repeat(500), 100).ends_with('…'));
    }
}
