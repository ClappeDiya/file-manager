//! Tamper-evident, append-only storage for security audit events (#357).
//!
//! # Model
//!
//! Every row of `audit_events` is a link in a SHA-256 hash chain:
//!
//! * `sequence` — strictly increasing, starts at 1, no gaps;
//! * `prev_hash` — the `hash` of entry `sequence - 1` (or [`GENESIS_HASH`]
//!   for entry 1);
//! * `hash` — `hex(SHA-256(canonical_json(entry)))`, where the canonical
//!   object holds `actor`, `details`, `event_type`, `id`, `prev_hash`,
//!   `resource`, `sequence`, `timestamp` and `v` (= [`CHAIN_VERSION`]).
//!
//! Canonical JSON ([`canonical_json`]) is a single object, keys sorted by
//! byte order, no whitespace, strings escaped minimally per RFC 8259
//! (`"` `\` `\b` `\f` `\n` `\r` `\t`, other controls as lowercase
//! `\u00xx`, everything else raw UTF-8) and integers in decimal. No floats
//! appear. It is byte-identical to Python's
//! `json.dumps(obj, sort_keys=True, separators=(",", ":"), ensure_ascii=False)`
//! so an external party can recompute the chain from an export.
//!
//! # Enforcement
//!
//! * SQLite triggers (migration v17) abort every `UPDATE` and `DELETE` on
//!   `audit_events` / `audit_anchors`, and abort any `INSERT` that does
//!   not extend the chain (wrong sequence, wrong `prev_hash`, reused id).
//! * All writes go through [`append_event`] / [`create_anchor`], which
//!   run inside `BEGIN IMMEDIATE` on the pool's single mutex-guarded
//!   connection, so the tail read and the insert can never interleave —
//!   not even with a second process sharing the database file.
//! * There is deliberately no retention / prune path. A full database
//!   reset keeps the audit tables (see `migrations::reset_database`).
//!
//! # Anchors
//!
//! An anchor is a signed checkpoint `{anchor_seq, last_sequence, last_hash,
//! count, created_at, key_id, algorithm, signature}`. The signature is
//! Ed25519 over the anchor's canonical JSON (with `"type":"ufop.audit.anchor"`
//! and `v`), made with a key held outside the database (env var, OS
//! keychain, or the encrypted settings store — see `security::audit`).
//! Anchors make truncation and wholesale re-writing of the chain
//! detectable: an attacker with raw DB access who drops the triggers can
//! re-hash the rows, but cannot re-sign the anchors. Ed25519 lets anyone
//! holding the public key (included in exports) verify them.
//!
//! Limitation: entries appended after the newest anchor are protected by
//! the chain and triggers only; anchor often (the app anchors at start-up
//! and hourly) and keep exports off-machine for strongest guarantees.

use crate::core::error::AppError;
use crate::core::types::AuditEvent;
use ring::signature::{Ed25519KeyPair, KeyPair, UnparsedPublicKey, ED25519};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// `prev_hash` of the first entry.
pub const GENESIS_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";
/// Version of the entry/anchor hashing scheme (part of every hash input).
pub const CHAIN_VERSION: i64 = 1;
/// Signature algorithm used for anchors.
pub const ANCHOR_ALGORITHM: &str = "ed25519";
/// `format` value in the export header line.
pub const EXPORT_FORMAT: &str = "ufop-audit-log";

/// Trigger names installed by migration v17. Verification reports whether
/// all of them are still present.
pub const APPEND_ONLY_TRIGGERS: [&str; 6] = [
    "audit_events_append_only_insert",
    "audit_events_append_only_update",
    "audit_events_append_only_delete",
    "audit_anchors_append_only_insert",
    "audit_anchors_append_only_update",
    "audit_anchors_append_only_delete",
];

// ── Canonical JSON ──

/// A value in a canonical JSON object. Only strings and integers occur in
/// audit hashing inputs, which keeps the encoding unambiguous.
#[derive(Debug, Clone, Copy)]
pub enum CanonValue<'a> {
    Str(&'a str),
    Int(i64),
}

fn push_json_string(out: &mut String, s: &str) {
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Serialize `fields` as a canonical JSON object (keys sorted by byte
/// order, no whitespace). Keys must be unique.
pub fn canonical_json(fields: &[(&str, CanonValue<'_>)]) -> String {
    let mut sorted: Vec<&(&str, CanonValue<'_>)> = fields.iter().collect();
    sorted.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    debug_assert!(sorted.windows(2).all(|w| w[0].0 != w[1].0), "duplicate canonical key");
    let mut out = String::with_capacity(256);
    out.push('{');
    for (i, (k, v)) in sorted.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        push_json_string(&mut out, k);
        out.push(':');
        match v {
            CanonValue::Str(s) => push_json_string(&mut out, s),
            CanonValue::Int(n) => out.push_str(&n.to_string()),
        }
    }
    out.push('}');
    out
}

fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

// ── Records ──

/// One audit entry as stored, including its chain fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditRecord {
    pub sequence: i64,
    pub id: String,
    pub timestamp: String,
    pub event_type: String,
    pub actor: String,
    pub resource: String,
    pub details: String,
    pub prev_hash: String,
    pub hash: String,
}

impl AuditRecord {
    /// The exact bytes hashed into `hash`.
    pub fn canonical_payload(&self) -> String {
        canonical_json(&[
            ("actor", CanonValue::Str(&self.actor)),
            ("details", CanonValue::Str(&self.details)),
            ("event_type", CanonValue::Str(&self.event_type)),
            ("id", CanonValue::Str(&self.id)),
            ("prev_hash", CanonValue::Str(&self.prev_hash)),
            ("resource", CanonValue::Str(&self.resource)),
            ("sequence", CanonValue::Int(self.sequence)),
            ("timestamp", CanonValue::Str(&self.timestamp)),
            ("v", CanonValue::Int(CHAIN_VERSION)),
        ])
    }

    /// Recompute the entry hash from its fields.
    pub fn compute_hash(&self) -> String {
        sha256_hex(self.canonical_payload().as_bytes())
    }

    /// Build a record linked after `prev_hash` and fill in its hash.
    pub fn link(sequence: i64, prev_hash: &str, event: &AuditEvent) -> Self {
        let mut rec = Self {
            sequence,
            id: event.id.to_string(),
            timestamp: event.timestamp.to_rfc3339_opts(chrono::SecondsFormat::Micros, true),
            event_type: event.event_type.clone(),
            actor: event.actor.clone(),
            resource: event.resource.clone(),
            details: event.details.clone(),
            prev_hash: prev_hash.to_string(),
            hash: String::new(),
        };
        rec.hash = rec.compute_hash();
        rec
    }
}

/// A signed checkpoint over the chain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditAnchor {
    pub anchor_seq: i64,
    pub last_sequence: i64,
    pub last_hash: String,
    pub count: i64,
    pub created_at: String,
    pub key_id: String,
    pub algorithm: String,
    pub signature: String,
}

impl AuditAnchor {
    /// The exact bytes covered by `signature`.
    pub fn signing_payload(&self) -> String {
        canonical_json(&[
            ("algorithm", CanonValue::Str(&self.algorithm)),
            ("anchor_seq", CanonValue::Int(self.anchor_seq)),
            ("count", CanonValue::Int(self.count)),
            ("created_at", CanonValue::Str(&self.created_at)),
            ("key_id", CanonValue::Str(&self.key_id)),
            ("last_hash", CanonValue::Str(&self.last_hash)),
            ("last_sequence", CanonValue::Int(self.last_sequence)),
            ("type", CanonValue::Str("ufop.audit.anchor")),
            ("v", CanonValue::Int(CHAIN_VERSION)),
        ])
    }
}

// ── Keys ──

/// Short identifier of a public key: first 16 hex chars of SHA-256(pk).
pub fn key_id_for(public_key: &[u8]) -> String {
    sha256_hex(public_key)[..16].to_string()
}

/// Ed25519 signing key for anchors. Never serialized.
pub struct AuditSigner {
    keypair: Ed25519KeyPair,
    verifier: AnchorVerifier,
}

impl std::fmt::Debug for AuditSigner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuditSigner").field("key_id", &self.verifier.key_id).finish_non_exhaustive()
    }
}

impl AuditSigner {
    /// Build from a 32-byte Ed25519 seed.
    pub fn from_seed(seed: &[u8]) -> Result<Self, AppError> {
        let keypair = Ed25519KeyPair::from_seed_unchecked(seed).map_err(|_| AppError::Encryption {
            message: "The audit signing key is malformed.".into(),
            advice: "The key must be a 32-byte Ed25519 seed (64 hex characters).".into(),
        })?;
        let mut pk = [0u8; 32];
        pk.copy_from_slice(keypair.public_key().as_ref());
        Ok(Self { keypair, verifier: AnchorVerifier::new(pk) })
    }

    /// A fresh random seed.
    pub fn generate_seed() -> zeroize::Zeroizing<[u8; 32]> {
        use rand::RngCore;
        let mut seed = zeroize::Zeroizing::new([0u8; 32]);
        rand::rngs::OsRng.fill_bytes(seed.as_mut());
        seed
    }

    pub fn verifier(&self) -> &AnchorVerifier {
        &self.verifier
    }

    pub fn key_id(&self) -> &str {
        &self.verifier.key_id
    }

    fn sign_hex(&self, msg: &[u8]) -> String {
        hex::encode(self.keypair.sign(msg).as_ref())
    }
}

/// Public half of the anchor key; all that verification needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnchorVerifier {
    pub key_id: String,
    pub public_key: [u8; 32],
}

impl AnchorVerifier {
    pub fn new(public_key: [u8; 32]) -> Self {
        Self { key_id: key_id_for(&public_key), public_key }
    }

    pub fn from_hex(public_key_hex: &str) -> Option<Self> {
        let raw = hex::decode(public_key_hex.trim()).ok()?;
        let pk: [u8; 32] = raw.try_into().ok()?;
        Some(Self::new(pk))
    }

    pub fn public_key_hex(&self) -> String {
        hex::encode(self.public_key)
    }

    /// `Err(reason)` when the anchor was not signed by this key.
    pub fn check(&self, anchor: &AuditAnchor) -> Result<(), String> {
        if anchor.algorithm != ANCHOR_ALGORITHM {
            return Err(format!("anchor #{} uses unsupported algorithm '{}'", anchor.anchor_seq, anchor.algorithm));
        }
        if anchor.key_id != self.key_id {
            return Err(format!(
                "anchor #{} was signed with key {}, not the trusted audit key {}",
                anchor.anchor_seq, anchor.key_id, self.key_id
            ));
        }
        let sig = hex::decode(&anchor.signature).map_err(|_| format!("anchor #{} signature is not valid hex", anchor.anchor_seq))?;
        UnparsedPublicKey::new(&ED25519, &self.public_key)
            .verify(anchor.signing_payload().as_bytes(), &sig)
            .map_err(|_| format!("anchor #{} signature does not verify (forged or altered)", anchor.anchor_seq))
    }
}

// ── Verification ──

/// Result of a full integrity check. Field names are the IPC contract
/// (`AuditVerification` in `src/lib/audit-log.ts`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditVerification {
    pub ok: bool,
    /// Entries whose sequence, link and hash all checked out.
    pub entries_checked: u64,
    /// First entry that is altered, missing (gap) or mis-linked.
    pub first_bad_sequence: Option<i64>,
    /// First anchor that failed its signature or linkage check.
    pub first_bad_anchor: Option<i64>,
    /// Every problem found, `; `-joined. `None` when `ok`.
    pub reason: Option<String>,
    pub anchors_verified: u64,
    pub anchors_total: u64,
    pub last_sequence: Option<i64>,
    pub last_hash: Option<String>,
    /// Whether the append-only triggers are all installed (`None` when
    /// verifying an export, where it does not apply).
    pub append_only_enforced: Option<bool>,
    /// Key the anchors were checked against.
    pub key_id: Option<String>,
    pub verified_at: String,
}

/// Incremental verifier shared by the database and export paths.
struct ChainWalker<'a> {
    expected: i64,
    prev_hash: String,
    checked: u64,
    last_hash: Option<String>,
    /// Anchors that passed the signature check, sorted by last_sequence.
    pending: Vec<&'a AuditAnchor>,
    next_anchor: usize,
    anchors_verified: u64,
    problems: Vec<String>,
    first_bad_sequence: Option<i64>,
    first_bad_anchor: Option<i64>,
    broken: bool,
}

impl<'a> ChainWalker<'a> {
    fn new(anchors: &'a [AuditAnchor], verifier: Option<&AnchorVerifier>) -> Self {
        let mut w = Self {
            expected: 1,
            prev_hash: GENESIS_HASH.to_string(),
            checked: 0,
            last_hash: None,
            pending: Vec::new(),
            next_anchor: 0,
            anchors_verified: 0,
            problems: Vec::new(),
            first_bad_sequence: None,
            first_bad_anchor: None,
            broken: false,
        };
        let mut by_seq: Vec<&AuditAnchor> = anchors.iter().collect();
        by_seq.sort_by_key(|a| a.anchor_seq);
        let mut prev_last = 0i64;
        for (i, a) in by_seq.iter().enumerate() {
            let expect_seq = i as i64 + 1;
            if a.anchor_seq != expect_seq {
                w.bad_anchor(a.anchor_seq, format!("anchor sequence gap: expected anchor #{expect_seq}, found #{} (anchors were removed)", a.anchor_seq));
            }
            if a.last_sequence < prev_last {
                w.bad_anchor(a.anchor_seq, format!("anchor #{} goes backwards (covers {} after an anchor covering {prev_last})", a.anchor_seq, a.last_sequence));
            }
            prev_last = prev_last.max(a.last_sequence);
        }
        match verifier {
            None if !anchors.is_empty() => {
                w.problems.push(format!("audit signing key unavailable; {} anchor(s) could not be verified", anchors.len()));
            }
            None => {}
            Some(v) => {
                for a in by_seq {
                    match v.check(a) {
                        Ok(()) if a.last_sequence >= 1 => w.pending.push(a),
                        Ok(()) => w.bad_anchor(a.anchor_seq, format!("anchor #{} covers no entries", a.anchor_seq)),
                        Err(reason) => w.bad_anchor(a.anchor_seq, reason),
                    }
                }
            }
        }
        w.pending.sort_by_key(|a| (a.last_sequence, a.anchor_seq));
        w
    }

    fn bad_anchor(&mut self, anchor_seq: i64, reason: String) {
        self.first_bad_anchor = Some(self.first_bad_anchor.map_or(anchor_seq, |b| b.min(anchor_seq)));
        self.problems.push(reason);
    }

    fn bad_entry(&mut self, sequence: i64, reason: String) {
        self.first_bad_sequence.get_or_insert(sequence);
        self.problems.push(reason);
        self.broken = true;
    }

    /// Feed the next entry in ascending sequence order. Returns `false`
    /// once the chain is broken (later entries cannot be trusted).
    fn push(&mut self, rec: &AuditRecord) -> bool {
        if self.broken {
            return false;
        }
        if rec.sequence != self.expected {
            if rec.sequence > self.expected {
                let missing = if rec.sequence == self.expected + 1 {
                    format!("{}", self.expected)
                } else {
                    format!("{}..{}", self.expected, rec.sequence - 1)
                };
                self.bad_entry(self.expected, format!("sequence gap: entry {missing} missing (deleted) before entry {}", rec.sequence));
            } else {
                self.bad_entry(rec.sequence, format!("sequence {} is duplicated or out of order (expected {})", rec.sequence, self.expected));
            }
            return false;
        }
        if rec.prev_hash != self.prev_hash {
            self.bad_entry(rec.sequence, format!("entry {} does not link to entry {} (prev_hash mismatch)", rec.sequence, rec.sequence - 1));
            return false;
        }
        if rec.compute_hash() != rec.hash {
            self.bad_entry(rec.sequence, format!("entry {} content does not match its hash (altered)", rec.sequence));
            return false;
        }
        while let Some(a) = self.pending.get(self.next_anchor).copied() {
            if a.last_sequence != rec.sequence {
                break;
            }
            self.next_anchor += 1;
            if a.last_hash != rec.hash {
                self.bad_anchor(a.anchor_seq, format!("anchor #{} expects entry {} to hash to {}, chain has {}", a.anchor_seq, a.last_sequence, a.last_hash, rec.hash));
            } else if a.count != rec.sequence {
                self.bad_anchor(a.anchor_seq, format!("anchor #{} records {} entries up to sequence {}", a.anchor_seq, a.count, a.last_sequence));
            } else {
                self.anchors_verified += 1;
            }
        }
        self.prev_hash = rec.hash.clone();
        self.last_hash = Some(rec.hash.clone());
        self.checked += 1;
        self.expected += 1;
        true
    }

    fn finish(mut self, anchors_total: usize, append_only: Option<bool>, verifier: Option<&AnchorVerifier>) -> AuditVerification {
        let last_good = self.expected - 1;
        let unreached: Vec<&AuditAnchor> = self.pending[self.next_anchor..].to_vec();
        if !self.broken {
            if let Some(a) = unreached.iter().max_by_key(|a| a.last_sequence) {
                self.first_bad_sequence.get_or_insert(last_good + 1);
                self.problems.push(format!(
                    "log ends at entry {last_good} but signed anchor #{} covers entries up to {} (entries {}..{} were removed)",
                    a.anchor_seq,
                    a.last_sequence,
                    last_good + 1,
                    a.last_sequence
                ));
                for a in &unreached {
                    self.first_bad_anchor.get_or_insert(a.anchor_seq);
                }
            }
        } else if !unreached.is_empty() {
            self.problems.push(format!("{} anchor(s) beyond the break could not be checked", unreached.len()));
        }
        if append_only == Some(false) {
            self.problems.push("append-only triggers are missing (the log was opened for modification)".into());
        }
        let ok = self.problems.is_empty();
        AuditVerification {
            ok,
            entries_checked: self.checked,
            first_bad_sequence: self.first_bad_sequence,
            first_bad_anchor: self.first_bad_anchor,
            reason: if ok { None } else { Some(self.problems.join("; ")) },
            anchors_verified: self.anchors_verified,
            anchors_total: anchors_total as u64,
            last_sequence: (last_good > 0).then_some(last_good),
            last_hash: self.last_hash,
            append_only_enforced: append_only,
            key_id: verifier.map(|v| v.key_id.clone()),
            verified_at: now_rfc3339(),
        }
    }
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, true)
}

// ── SQL ──

const SELECT_RECORD: &str = "SELECT sequence, id, timestamp, event_type, actor, resource, details, prev_hash, hash FROM audit_events";
const SELECT_ANCHOR: &str =
    "SELECT anchor_seq, last_sequence, last_hash, entry_count, created_at, key_id, algorithm, signature FROM audit_anchors";

fn row_to_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<AuditRecord> {
    Ok(AuditRecord {
        sequence: row.get(0)?,
        id: row.get(1)?,
        timestamp: row.get(2)?,
        event_type: row.get(3)?,
        actor: row.get(4)?,
        resource: row.get(5)?,
        details: row.get(6)?,
        prev_hash: row.get::<_, Option<String>>(7)?.unwrap_or_default(),
        hash: row.get::<_, Option<String>>(8)?.unwrap_or_default(),
    })
}

fn row_to_anchor(row: &rusqlite::Row<'_>) -> rusqlite::Result<AuditAnchor> {
    Ok(AuditAnchor {
        anchor_seq: row.get(0)?,
        last_sequence: row.get(1)?,
        last_hash: row.get(2)?,
        count: row.get(3)?,
        created_at: row.get(4)?,
        key_id: row.get(5)?,
        algorithm: row.get(6)?,
        signature: row.get(7)?,
    })
}

/// `(sequence, hash)` of the newest entry.
pub fn chain_tail(conn: &Connection) -> Result<Option<(i64, String)>, AppError> {
    Ok(conn
        .query_row(
            "SELECT sequence, hash FROM audit_events WHERE sequence IS NOT NULL ORDER BY sequence DESC LIMIT 1",
            [],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?.unwrap_or_default())),
        )
        .optional()?)
}

fn insert_record(conn: &Connection, rec: &AuditRecord) -> Result<(), AppError> {
    conn.execute(
        "INSERT INTO audit_events (id, timestamp, event_type, actor, resource, details, sequence, prev_hash, hash)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![rec.id, rec.timestamp, rec.event_type, rec.actor, rec.resource, rec.details, rec.sequence, rec.prev_hash, rec.hash],
    )?;
    Ok(())
}

/// Append one event to the chain. The tail read and the insert share one
/// `BEGIN IMMEDIATE` transaction, so concurrent writers (even in another
/// process) are serialized and can never fork the chain.
pub fn append_event(conn: &Connection, event: &AuditEvent) -> Result<AuditRecord, AppError> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let (seq, prev) = chain_tail(&tx)?.unwrap_or((0, GENESIS_HASH.to_string()));
    let rec = AuditRecord::link(seq + 1, &prev, event);
    insert_record(&tx, &rec)?;
    tx.commit()?;
    Ok(rec)
}

/// All anchors in creation order.
pub fn list_anchors(conn: &Connection) -> Result<Vec<AuditAnchor>, AppError> {
    let mut stmt = conn.prepare(&format!("{SELECT_ANCHOR} ORDER BY anchor_seq ASC"))?;
    let rows = stmt.query_map([], row_to_anchor)?.collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn latest_anchor(conn: &Connection) -> Result<Option<AuditAnchor>, AppError> {
    Ok(conn.query_row(&format!("{SELECT_ANCHOR} ORDER BY anchor_seq DESC LIMIT 1"), [], row_to_anchor).optional()?)
}

/// Whether all append-only triggers are installed.
pub fn append_only_enforced(conn: &Connection) -> Result<bool, AppError> {
    let mut stmt = conn.prepare("SELECT name FROM sqlite_master WHERE type = 'trigger'")?;
    let names: Vec<String> = stmt.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
    Ok(APPEND_ONLY_TRIGGERS.iter().all(|t| names.iter().any(|n| n == t)))
}

/// Walk entries with `sequence > after` in order, feeding `f` until it
/// returns `false`. Rows that cannot be decoded are reported as `Err`.
fn walk_entries(
    conn: &Connection,
    after: i64,
    mut f: impl FnMut(Result<AuditRecord, i64>) -> bool,
) -> Result<(), AppError> {
    let mut stmt = conn.prepare(&format!("{SELECT_RECORD} WHERE sequence > ?1 ORDER BY sequence ASC"))?;
    let mut rows = stmt.query([after])?;
    while let Some(row) = rows.next()? {
        let item = row_to_record(row).map_err(|_| row.get::<_, i64>(0).unwrap_or(-1));
        if !f(item) {
            break;
        }
    }
    Ok(())
}

/// Recompute the whole chain, check for gaps and verify every anchor.
pub fn verify(conn: &Connection, verifier: Option<&AnchorVerifier>) -> Result<AuditVerification, AppError> {
    // Read a consistent snapshot even if another connection is appending.
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Deferred)?;
    let anchors = list_anchors(&tx)?;
    let append_only = append_only_enforced(&tx)?;
    let unsequenced: i64 = tx.query_row("SELECT COUNT(*) FROM audit_events WHERE sequence IS NULL", [], |r| r.get(0))?;
    let mut walker = ChainWalker::new(&anchors, verifier);
    if unsequenced > 0 {
        walker.problems.push(format!("{unsequenced} audit row(s) have no sequence (inserted outside the hash chain)"));
    }
    walk_entries(&tx, 0, |item| match item {
        Ok(rec) => walker.push(&rec),
        Err(seq) => {
            walker.bad_entry(seq, format!("entry {seq} has malformed columns (altered)"));
            false
        }
    })?;
    let result = walker.finish(anchors.len(), Some(append_only), verifier);
    tx.commit()?;
    Ok(result)
}

/// Sign a checkpoint over the current tail. The entries added since the
/// previous anchor are re-verified first, so a tampered chain is never
/// signed. Fails when the log is empty.
pub fn create_anchor(conn: &Connection, signer: &AuditSigner) -> Result<AuditAnchor, AppError> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let Some((last_sequence, last_hash)) = chain_tail(&tx)? else {
        return Err(AppError::validation("The security audit log is empty; there is nothing to anchor yet."));
    };
    let prev = latest_anchor(&tx)?;
    let (start_seq, start_hash) = match &prev {
        Some(a) => (a.last_sequence, a.last_hash.clone()),
        None => (0, GENESIS_HASH.to_string()),
    };
    if start_seq > 0 {
        let h: Option<String> = tx
            .query_row("SELECT hash FROM audit_events WHERE sequence = ?1", [start_seq], |r| r.get(0))
            .optional()?
            .flatten();
        if h.as_deref() != Some(start_hash.as_str()) {
            return Err(refuse_anchor(format!("entry {start_seq} no longer matches the previous anchor")));
        }
    }
    let mut expected = start_seq + 1;
    let mut prev_hash = start_hash;
    let mut failure: Option<String> = None;
    walk_entries(&tx, start_seq, |item| {
        let ok = match &item {
            Ok(rec) => rec.sequence == expected && rec.prev_hash == prev_hash && rec.compute_hash() == rec.hash,
            Err(_) => false,
        };
        if !ok {
            failure = Some(format!("entry {expected} fails verification"));
            return false;
        }
        if let Ok(rec) = item {
            prev_hash = rec.hash;
        }
        expected += 1;
        true
    })?;
    if let Some(f) = failure {
        return Err(refuse_anchor(f));
    }
    let count: i64 = tx.query_row("SELECT COUNT(*) FROM audit_events", [], |r| r.get(0))?;
    if count != last_sequence {
        return Err(refuse_anchor(format!("{count} rows but the chain ends at sequence {last_sequence}")));
    }
    let mut anchor = AuditAnchor {
        anchor_seq: prev.as_ref().map_or(1, |a| a.anchor_seq + 1),
        last_sequence,
        last_hash,
        count,
        created_at: now_rfc3339(),
        key_id: signer.key_id().to_string(),
        algorithm: ANCHOR_ALGORITHM.to_string(),
        signature: String::new(),
    };
    anchor.signature = signer.sign_hex(anchor.signing_payload().as_bytes());
    tx.execute(
        "INSERT INTO audit_anchors (anchor_seq, last_sequence, last_hash, entry_count, created_at, key_id, algorithm, signature)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            anchor.anchor_seq,
            anchor.last_sequence,
            anchor.last_hash,
            anchor.count,
            anchor.created_at,
            anchor.key_id,
            anchor.algorithm,
            anchor.signature
        ],
    )?;
    tx.commit()?;
    Ok(anchor)
}

fn refuse_anchor(detail: String) -> AppError {
    AppError::State {
        message: format!("Refusing to sign an audit checkpoint: the log failed verification ({detail})."),
        advice: "Run \"Verify integrity\" in Settings → Security audit log and keep an export for investigation.".into(),
    }
}

/// Number of entries newer than the latest anchor.
pub fn unanchored_count(conn: &Connection) -> Result<i64, AppError> {
    let anchored = latest_anchor(conn)?.map_or(0, |a| a.last_sequence);
    Ok(conn.query_row("SELECT COUNT(*) FROM audit_events WHERE sequence > ?1", [anchored], |r| r.get(0))?)
}

// ── Export ──

/// One line of a JSON-lines export.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ExportLine {
    Header(ExportHeader),
    Entry(AuditRecord),
    Anchor(AuditAnchor),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportHeader {
    pub format: String,
    pub version: i64,
    pub exported_at: String,
    pub hash_algorithm: String,
    pub genesis_prev_hash: String,
    /// Human-readable description of the hash and signature inputs.
    pub canonicalization: String,
    pub anchor_algorithm: String,
    /// Ed25519 public key (hex) that signed the anchors; pin it
    /// out-of-band rather than trusting this copy.
    pub public_key: Option<String>,
    pub key_id: Option<String>,
    pub entry_count: i64,
    pub anchor_count: i64,
}

const CANONICALIZATION: &str = "entry.hash = hex(sha256(J({actor,details,event_type,id,prev_hash,resource,sequence,timestamp,v:1}))); \
anchor.signature = hex(ed25519(J({algorithm,anchor_seq,count,created_at,key_id,last_hash,last_sequence,type:\"ufop.audit.anchor\",v:1}))); \
J = JSON object, keys sorted, no whitespace, minimal RFC 8259 escaping, raw UTF-8 \
(Python: json.dumps(o, sort_keys=True, separators=(',', ':'), ensure_ascii=False))";

/// Export the full log (header, entries in order, anchors) as JSON lines.
pub fn export_jsonl(conn: &Connection, verifier: Option<&AnchorVerifier>) -> Result<String, AppError> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Deferred)?;
    let anchors = list_anchors(&tx)?;
    let entry_count: i64 = tx.query_row("SELECT COUNT(*) FROM audit_events", [], |r| r.get(0))?;
    let header = ExportLine::Header(ExportHeader {
        format: EXPORT_FORMAT.into(),
        version: CHAIN_VERSION,
        exported_at: now_rfc3339(),
        hash_algorithm: "sha256".into(),
        genesis_prev_hash: GENESIS_HASH.into(),
        canonicalization: CANONICALIZATION.into(),
        anchor_algorithm: ANCHOR_ALGORITHM.into(),
        public_key: verifier.map(|v| v.public_key_hex()),
        key_id: verifier.map(|v| v.key_id.clone()),
        entry_count,
        anchor_count: anchors.len() as i64,
    });
    let mut out = serde_json::to_string(&header)?;
    out.push('\n');
    let mut stmt = tx.prepare(&format!("{SELECT_RECORD} ORDER BY sequence ASC"))?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        // Malformed rows are exported as far as they decode; verification
        // of the export will flag them.
        if let Ok(rec) = row_to_record(row) {
            out.push_str(&serde_json::to_string(&ExportLine::Entry(rec))?);
            out.push('\n');
        }
    }
    drop(rows);
    drop(stmt);
    for a in anchors {
        out.push_str(&serde_json::to_string(&ExportLine::Anchor(a))?);
        out.push('\n');
    }
    tx.commit()?;
    Ok(out)
}

/// Verify a JSON-lines export. Anchors are checked against
/// `trusted_public_key` when given, else against the key in the header
/// (which only proves self-consistency — pin the key out-of-band).
pub fn verify_jsonl(text: &str, trusted_public_key: Option<&AnchorVerifier>) -> AuditVerification {
    let mut header: Option<ExportHeader> = None;
    let mut entries: Vec<AuditRecord> = Vec::new();
    let mut anchors: Vec<AuditAnchor> = Vec::new();
    let mut parse_errors: Vec<String> = Vec::new();
    for (i, line) in text.lines().enumerate().filter(|(_, l)| !l.trim().is_empty()) {
        match serde_json::from_str::<ExportLine>(line) {
            Ok(ExportLine::Header(h)) => header = Some(h),
            Ok(ExportLine::Entry(e)) => entries.push(e),
            Ok(ExportLine::Anchor(a)) => anchors.push(a),
            Err(e) => parse_errors.push(format!("line {}: {e}", i + 1)),
        }
    }
    let header_key = header.as_ref().and_then(|h| h.public_key.as_deref()).and_then(AnchorVerifier::from_hex);
    let verifier = trusted_public_key.cloned().or(header_key);
    let mut walker = ChainWalker::new(&anchors, verifier.as_ref());
    if header.as_ref().map(|h| h.format.as_str()) != Some(EXPORT_FORMAT) {
        walker.problems.push("missing or unrecognized export header".into());
    }
    walker.problems.extend(parse_errors);
    entries.sort_by_key(|e| e.sequence);
    for e in &entries {
        if !walker.push(e) {
            break;
        }
    }
    if let Some(h) = &header {
        if h.entry_count != entries.len() as i64 && !walker.broken {
            walker.problems.push(format!("header lists {} entries but the export contains {}", h.entry_count, entries.len()));
        }
    }
    walker.finish(anchors.len(), None, verifier.as_ref())
}

// ── Migration support (v17) ──

/// Install the chain on an existing `audit_events` table: add the chain
/// columns, hash the existing rows in (timestamp, rowid) order so old logs
/// become the start of the chain, then create the anchors table, unique
/// index and append-only triggers. Idempotent, so it also survives a
/// database reset that keeps the audit tables.
pub fn migrate_v17(conn: &Connection) -> Result<(), AppError> {
    let cols: Vec<String> = {
        let mut stmt = conn.prepare("PRAGMA table_info(audit_events)")?;
        let names = stmt.query_map([], |r| r.get::<_, String>(1))?.collect::<Result<_, _>>()?;
        names
    };
    for (col, ty) in [("sequence", "INTEGER"), ("prev_hash", "TEXT"), ("hash", "TEXT")] {
        if !cols.iter().any(|c| c == col) {
            conn.execute_batch(&format!("ALTER TABLE audit_events ADD COLUMN {col} {ty};"))?;
        }
    }
    let triggers_installed: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'trigger' AND name = 'audit_events_append_only_update')",
        [],
        |r| r.get(0),
    )?;
    if !triggers_installed {
        let legacy: Vec<(i64, AuditRecord)> = {
            let mut stmt = conn.prepare(
                "SELECT rowid, id, timestamp, event_type, actor, resource, details FROM audit_events
                 WHERE sequence IS NULL ORDER BY timestamp ASC, rowid ASC",
            )?;
            let rows = stmt
                .query_map([], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        AuditRecord {
                            sequence: 0,
                            id: r.get(1)?,
                            timestamp: r.get(2)?,
                            event_type: r.get(3)?,
                            actor: r.get(4)?,
                            resource: r.get(5)?,
                            details: r.get(6)?,
                            prev_hash: String::new(),
                            hash: String::new(),
                        },
                    ))
                })?
                .collect::<Result<_, _>>()?;
            rows
        };
        let (mut seq, mut prev) = chain_tail(conn)?.unwrap_or((0, GENESIS_HASH.to_string()));
        for (rowid, mut rec) in legacy {
            seq += 1;
            rec.sequence = seq;
            rec.prev_hash = prev;
            rec.hash = rec.compute_hash();
            conn.execute(
                "UPDATE audit_events SET sequence = ?1, prev_hash = ?2, hash = ?3 WHERE rowid = ?4",
                params![rec.sequence, rec.prev_hash, rec.hash, rowid],
            )?;
            prev = rec.hash;
        }
    }
    conn.execute_batch(V17_CHAIN_SQL)?;
    Ok(())
}

const V17_CHAIN_SQL: &str = r#"
CREATE UNIQUE INDEX IF NOT EXISTS idx_audit_sequence ON audit_events(sequence);

CREATE TABLE IF NOT EXISTS audit_anchors (
    anchor_seq     INTEGER PRIMARY KEY NOT NULL,
    last_sequence  INTEGER NOT NULL,
    last_hash      TEXT NOT NULL,
    entry_count    INTEGER NOT NULL,
    created_at     TEXT NOT NULL,
    key_id         TEXT NOT NULL,
    algorithm      TEXT NOT NULL,
    signature      TEXT NOT NULL
);

CREATE TRIGGER IF NOT EXISTS audit_events_append_only_update
BEFORE UPDATE ON audit_events
BEGIN
    SELECT RAISE(ABORT, 'audit_events is append-only: rows cannot be modified');
END;

CREATE TRIGGER IF NOT EXISTS audit_events_append_only_delete
BEFORE DELETE ON audit_events
BEGIN
    SELECT RAISE(ABORT, 'audit_events is append-only: rows cannot be deleted');
END;

CREATE TRIGGER IF NOT EXISTS audit_events_append_only_insert
BEFORE INSERT ON audit_events
WHEN NEW.sequence IS NULL OR NEW.prev_hash IS NULL OR NEW.hash IS NULL
  OR NEW.sequence <> COALESCE((SELECT MAX(sequence) FROM audit_events), 0) + 1
  OR NEW.prev_hash <> COALESCE(
        (SELECT hash FROM audit_events WHERE sequence = (SELECT MAX(sequence) FROM audit_events)),
        '0000000000000000000000000000000000000000000000000000000000000000')
  OR EXISTS (SELECT 1 FROM audit_events WHERE id = NEW.id)
BEGIN
    SELECT RAISE(ABORT, 'audit_events is append-only: new rows must extend the hash chain');
END;

CREATE TRIGGER IF NOT EXISTS audit_anchors_append_only_update
BEFORE UPDATE ON audit_anchors
BEGIN
    SELECT RAISE(ABORT, 'audit_anchors is append-only: anchors cannot be modified');
END;

CREATE TRIGGER IF NOT EXISTS audit_anchors_append_only_delete
BEFORE DELETE ON audit_anchors
BEGIN
    SELECT RAISE(ABORT, 'audit_anchors is append-only: anchors cannot be deleted');
END;

CREATE TRIGGER IF NOT EXISTS audit_anchors_append_only_insert
BEFORE INSERT ON audit_anchors
WHEN NEW.anchor_seq IS NULL
  OR NEW.anchor_seq <> COALESCE((SELECT MAX(anchor_seq) FROM audit_anchors), 0) + 1
  OR NEW.last_sequence < COALESCE((SELECT MAX(last_sequence) FROM audit_anchors), 0)
  OR NEW.last_sequence > COALESCE((SELECT MAX(sequence) FROM audit_events), 0)
BEGIN
    SELECT RAISE(ABORT, 'audit_anchors is append-only: new anchors must follow the latest one');
END;
"#;
