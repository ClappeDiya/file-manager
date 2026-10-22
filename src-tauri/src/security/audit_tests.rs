//! Tests for the tamper-evident security audit log (#357).

use super::*;
use crate::security::audit_chain::{
    self as chain, canonical_json, AuditRecord, CanonValue, ExportLine, GENESIS_HASH,
};
use crate::security::settings_store::SettingsStore;
use crate::storage::DbPool;

const SEED: [u8; 32] = [7u8; 32];

async fn setup() -> (Repository, AuditTrail) {
    let repo = Repository::open_in_memory().await.unwrap();
    let trail = AuditTrail::with_seed(repo.pool().clone(), &SEED).unwrap();
    (repo, trail)
}

async fn log_n(repo: &Repository, n: usize) {
    for i in 0..n {
        SecurityAudit::log(repo, "test.event", "user", &format!("res-{i}"), &format!("details {i}")).await;
    }
}

/// Run raw SQL on the pool, returning the SQLite error text on failure.
async fn raw(repo: &Repository, sql: &'static str) -> Result<usize, String> {
    repo.pool()
        .execute(move |c| Ok(c.execute(sql, []).map_err(|e| e.to_string())))
        .await
        .unwrap()
}

async fn drop_triggers(repo: &Repository) {
    repo.pool()
        .execute(|c| {
            for t in chain::APPEND_ONLY_TRIGGERS {
                c.execute_batch(&format!("DROP TRIGGER IF EXISTS {t}"))?;
            }
            Ok(())
        })
        .await
        .unwrap();
}

// ── Canonical form ──

#[test]
fn canonical_json_sorts_keys_and_escapes_minimally() {
    let s = canonical_json(&[
        ("b", CanonValue::Str("x\"\\\n\u{1}\u{7f}é")),
        ("a", CanonValue::Int(-5)),
        ("_", CanonValue::Str("")),
    ]);
    assert_eq!(s, "{\"_\":\"\",\"a\":-5,\"b\":\"x\\\"\\\\\\n\\u0001\u{7f}é\"}");
}

/// Pinned against Python:
/// `sha256(json.dumps(rec, sort_keys=True, separators=(",", ":"), ensure_ascii=False))`.
#[test]
fn entry_hash_matches_external_reference_vector() {
    let rec = AuditRecord {
        sequence: 1,
        id: "00000000-0000-4000-8000-000000000001".into(),
        timestamp: "2026-01-02T03:04:05.000006Z".into(),
        event_type: "security.auth.master_password".into(),
        actor: "user".into(),
        resource: "master_password".into(),
        details: "line1\nq\"\\ \u{1} é".into(),
        prev_hash: GENESIS_HASH.into(),
        hash: String::new(),
    };
    assert_eq!(rec.compute_hash(), "0da67b97fc98856adf3640fafd02e230d43facfd42c7a40a2e3e9f37989e9794");
}

// ── Chain ──

#[tokio::test]
async fn appends_form_a_valid_chain() {
    let (repo, trail) = setup().await;
    log_n(&repo, 5).await;
    let v = trail.verify().await.unwrap();
    assert!(v.ok, "{v:?}");
    assert_eq!(v.entries_checked, 5);
    assert_eq!(v.last_sequence, Some(5));
    assert_eq!(v.first_bad_sequence, None);
    assert_eq!(v.append_only_enforced, Some(true));

    // Existing read API still works, newest first.
    let events = repo.query_audit_events(10, 0).await.unwrap();
    assert_eq!(events.len(), 5);
    assert_eq!(events[0].resource, "res-4");
    assert_eq!(events[0].event_type, "security.test.event");
}

#[tokio::test]
async fn empty_log_verifies() {
    let (_repo, trail) = setup().await;
    let v = trail.verify().await.unwrap();
    assert!(v.ok);
    assert_eq!(v.entries_checked, 0);
    assert_eq!(v.last_sequence, None);
    assert!(trail.create_anchor().await.is_err(), "nothing to anchor");
    assert!(trail.anchor_if_new().await.unwrap().is_none());
}

#[tokio::test]
async fn raw_update_and_delete_are_blocked_by_triggers() {
    let (repo, trail) = setup().await;
    log_n(&repo, 3).await;
    trail.create_anchor().await.unwrap();

    let e = raw(&repo, "UPDATE audit_events SET details = 'forged' WHERE sequence = 2").await.unwrap_err();
    assert!(e.contains("append-only"), "{e}");
    let e = raw(&repo, "DELETE FROM audit_events WHERE sequence = 3").await.unwrap_err();
    assert!(e.contains("append-only"), "{e}");
    let e = raw(&repo, "DELETE FROM audit_events").await.unwrap_err();
    assert!(e.contains("append-only"), "{e}");
    let e = raw(&repo, "UPDATE audit_anchors SET signature = '00'").await.unwrap_err();
    assert!(e.contains("append-only"), "{e}");
    let e = raw(&repo, "DELETE FROM audit_anchors").await.unwrap_err();
    assert!(e.contains("append-only"), "{e}");

    // Inserts that do not extend the chain are rejected too.
    let e = raw(
        &repo,
        "INSERT INTO audit_events (id, timestamp, event_type, actor, resource, details)
         VALUES ('x', 'now', 't', 'a', 'r', 'd')",
    )
    .await
    .unwrap_err();
    assert!(e.contains("hash chain"), "{e}");
    let e = raw(
        &repo,
        "INSERT INTO audit_events (id, timestamp, event_type, actor, resource, details, sequence, prev_hash, hash)
         VALUES ('y', 'now', 't', 'a', 'r', 'd', 4, 'bogus', 'bogus')",
    )
    .await
    .unwrap_err();
    assert!(e.contains("hash chain"), "{e}");
    // REPLACE would delete the old row without firing the DELETE trigger;
    // the insert trigger refuses a reused id.
    let e = raw(
        &repo,
        "INSERT OR REPLACE INTO audit_events (id, timestamp, event_type, actor, resource, details, sequence, prev_hash, hash)
         SELECT id, timestamp, event_type, actor, resource, 'forged', 4, (SELECT hash FROM audit_events WHERE sequence = 3), 'h'
         FROM audit_events WHERE sequence = 1",
    )
    .await
    .unwrap_err();
    assert!(e.contains("hash chain"), "{e}");

    let v = trail.verify().await.unwrap();
    assert!(v.ok, "{v:?}");
    assert_eq!(v.entries_checked, 3);
    assert_eq!(v.anchors_verified, 1);
}

#[tokio::test]
async fn altered_row_is_pinpointed_when_triggers_are_dropped() {
    let (repo, trail) = setup().await;
    log_n(&repo, 6).await;
    drop_triggers(&repo).await;
    raw(&repo, "UPDATE audit_events SET details = 'nothing to see here' WHERE sequence = 4").await.unwrap();

    let v = trail.verify().await.unwrap();
    assert!(!v.ok);
    assert_eq!(v.first_bad_sequence, Some(4));
    assert_eq!(v.entries_checked, 3);
    assert_eq!(v.append_only_enforced, Some(false));
    let reason = v.reason.unwrap();
    assert!(reason.contains("entry 4") && reason.contains("altered"), "{reason}");
    assert!(reason.contains("triggers are missing"), "{reason}");
}

#[tokio::test]
async fn deleted_row_is_detected_as_a_gap() {
    let (repo, trail) = setup().await;
    log_n(&repo, 5).await;
    drop_triggers(&repo).await;
    raw(&repo, "DELETE FROM audit_events WHERE sequence = 2").await.unwrap();

    let v = trail.verify().await.unwrap();
    assert!(!v.ok);
    assert_eq!(v.first_bad_sequence, Some(2));
    assert_eq!(v.entries_checked, 1);
    assert!(v.reason.unwrap().contains("sequence gap: entry 2 missing"));
}

#[tokio::test]
async fn rehashed_chain_is_caught_by_the_anchor() {
    let (repo, trail) = setup().await;
    log_n(&repo, 4).await;
    trail.create_anchor().await.unwrap();
    drop_triggers(&repo).await;

    // A capable attacker alters entry 2 and re-hashes everything after it.
    repo.pool()
        .execute(|c| {
            c.execute("UPDATE audit_events SET details = 'rewritten' WHERE sequence = 2", [])?;
            let mut prev: String = c.query_row("SELECT hash FROM audit_events WHERE sequence = 1", [], |r| r.get(0))?;
            for seq in 2..=4i64 {
                let mut rec: AuditRecord = c.query_row(
                    "SELECT sequence, id, timestamp, event_type, actor, resource, details FROM audit_events WHERE sequence = ?1",
                    [seq],
                    |r| {
                        Ok(AuditRecord {
                            sequence: r.get(0)?,
                            id: r.get(1)?,
                            timestamp: r.get(2)?,
                            event_type: r.get(3)?,
                            actor: r.get(4)?,
                            resource: r.get(5)?,
                            details: r.get(6)?,
                            prev_hash: String::new(),
                            hash: String::new(),
                        })
                    },
                )?;
                rec.prev_hash = prev;
                rec.hash = rec.compute_hash();
                c.execute(
                    "UPDATE audit_events SET prev_hash = ?1, hash = ?2 WHERE sequence = ?3",
                    rusqlite::params![rec.prev_hash, rec.hash, seq],
                )?;
                prev = rec.hash;
            }
            Ok(())
        })
        .await
        .unwrap();

    let v = trail.verify().await.unwrap();
    assert!(!v.ok);
    assert_eq!(v.first_bad_anchor, Some(1));
    assert_eq!(v.anchors_verified, 0);
    assert!(v.reason.unwrap().contains("anchor #1 expects entry 4"));
}

#[tokio::test]
async fn truncated_tail_is_caught_by_the_anchor() {
    let (repo, trail) = setup().await;
    log_n(&repo, 5).await;
    trail.create_anchor().await.unwrap();
    drop_triggers(&repo).await;
    raw(&repo, "DELETE FROM audit_events WHERE sequence >= 4").await.unwrap();

    let v = trail.verify().await.unwrap();
    assert!(!v.ok);
    assert_eq!(v.entries_checked, 3);
    assert_eq!(v.first_bad_sequence, Some(4));
    assert_eq!(v.first_bad_anchor, Some(1));
    assert!(v.reason.unwrap().contains("entries 4..5 were removed"));
}

#[tokio::test]
async fn forged_anchor_fails_signature_check() {
    let (repo, trail) = setup().await;
    log_n(&repo, 3).await;
    let a = trail.create_anchor().await.unwrap();
    assert_eq!((a.anchor_seq, a.last_sequence, a.count), (1, 3, 3));
    log_n(&repo, 2).await;

    // Forge anchor #2 with a made-up signature (bypassing the triggers).
    drop_triggers(&repo).await;
    repo.pool()
        .execute(move |c| {
            let (h,): (String,) = c.query_row("SELECT hash FROM audit_events WHERE sequence = 5", [], |r| Ok((r.get(0)?,)))?;
            c.execute(
                "INSERT INTO audit_anchors VALUES (2, 5, ?1, 5, '2026-01-01T00:00:00Z', ?2, 'ed25519', ?3)",
                rusqlite::params![h, a.key_id, "ab".repeat(64)],
            )?;
            Ok(())
        })
        .await
        .unwrap();

    let v = trail.verify().await.unwrap();
    assert!(!v.ok);
    assert_eq!(v.first_bad_anchor, Some(2));
    assert_eq!(v.first_bad_sequence, None);
    assert_eq!(v.anchors_verified, 1);
    assert_eq!(v.anchors_total, 2);
    assert!(v.reason.unwrap().contains("anchor #2 signature does not verify"));
}

#[tokio::test]
async fn anchor_signed_by_another_key_is_rejected() {
    let (repo, _trail) = setup().await;
    log_n(&repo, 2).await;
    let other = AuditTrail::with_seed(repo.pool().clone(), &[9u8; 32]).unwrap();
    other.create_anchor().await.unwrap();

    let trail = AuditTrail::with_seed(repo.pool().clone(), &SEED).unwrap();
    let v = trail.verify().await.unwrap();
    assert!(!v.ok);
    assert_eq!(v.first_bad_anchor, Some(1));
    assert!(v.reason.unwrap().contains("not the trusted audit key"));
}

#[tokio::test]
async fn anchoring_refuses_a_tampered_chain() {
    let (repo, trail) = setup().await;
    log_n(&repo, 3).await;
    drop_triggers(&repo).await;
    raw(&repo, "UPDATE audit_events SET actor = 'mallory' WHERE sequence = 2").await.unwrap();
    let err = trail.create_anchor().await.unwrap_err();
    assert!(err.to_string().contains("Refusing to sign"), "{err}");
}

#[tokio::test]
async fn anchor_if_new_only_anchors_new_entries() {
    let (repo, trail) = setup().await;
    log_n(&repo, 2).await;
    assert_eq!(trail.anchor_if_new().await.unwrap().unwrap().last_sequence, 2);
    assert!(trail.anchor_if_new().await.unwrap().is_none());
    log_n(&repo, 1).await;
    let a = trail.anchor_if_new().await.unwrap().unwrap();
    assert_eq!((a.anchor_seq, a.last_sequence), (2, 3));

    let s = trail.status().await.unwrap();
    assert_eq!((s.entries, s.anchors, s.unanchored_entries), (3, 2, 0));
    assert!(s.append_only_enforced);
    assert_eq!(s.key_source, Some(KeySource::Environment));
    assert_eq!(s.key_id.as_deref(), Some(a.key_id.as_str()));
    let v = trail.verify().await.unwrap();
    assert!(v.ok && v.anchors_verified == 2, "{v:?}");
}

#[tokio::test]
async fn concurrent_appends_keep_a_valid_chain() {
    let (repo, trail) = setup().await;
    let mut tasks = Vec::new();
    for i in 0..40 {
        let r = repo.clone();
        tasks.push(tokio::spawn(async move {
            SecurityAudit::log(&r, "concurrent", "user", &format!("task-{i}"), "").await;
        }));
    }
    for t in tasks {
        t.await.unwrap();
    }
    let v = trail.verify().await.unwrap();
    assert!(v.ok, "{v:?}");
    assert_eq!(v.entries_checked, 40);
}

/// Two connections to one file stand in for two processes (desktop app and
/// ufop-server): `BEGIN IMMEDIATE` must serialize their appends.
#[tokio::test]
async fn concurrent_appends_from_two_connections_keep_a_valid_chain() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("audit.db");
    let a = Repository::new(DbPool::open(&path).unwrap()).await.unwrap();
    let b = Repository::new(DbPool::open(&path).unwrap()).await.unwrap();
    let mut tasks = Vec::new();
    for i in 0..30 {
        let r = if i % 2 == 0 { a.clone() } else { b.clone() };
        tasks.push(tokio::spawn(async move {
            let event = AuditEvent {
                id: Uuid::new_v4(),
                timestamp: Utc::now(),
                event_type: "security.concurrent".into(),
                actor: "user".into(),
                resource: format!("r{i}"),
                details: String::new(),
            };
            r.log_audit_event(event).await.unwrap();
        }));
    }
    for t in tasks {
        t.await.unwrap();
    }
    let trail = AuditTrail::with_seed(a.pool().clone(), &SEED).unwrap();
    let v = trail.verify().await.unwrap();
    assert!(v.ok, "{v:?}");
    assert_eq!(v.entries_checked, 30);
}

// ── Export ──

#[tokio::test]
async fn export_round_trips_and_detects_tampering() {
    let (repo, trail) = setup().await;
    log_n(&repo, 4).await;
    trail.create_anchor().await.unwrap();
    let text = trail.export_jsonl().await.unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 1 + 4 + 1);
    let header: ExportLine = serde_json::from_str(lines[0]).unwrap();
    let ExportLine::Header(h) = header else { panic!("first line must be the header") };
    assert_eq!((h.entry_count, h.anchor_count), (4, 1));
    let pk = h.public_key.clone().unwrap();
    assert!(lines[1].contains("\"type\":\"entry\"") && lines[1].contains("\"hash\":"));
    assert!(lines[5].contains("\"type\":\"anchor\""));

    let trusted = chain::AnchorVerifier::from_hex(&pk).unwrap();
    let v = chain::verify_jsonl(&text, Some(&trusted));
    assert!(v.ok, "{v:?}");
    assert_eq!((v.entries_checked, v.anchors_verified), (4, 1));

    // Edit an entry in the export.
    let tampered = text.replacen("details 2", "details X", 1);
    let v = chain::verify_jsonl(&tampered, Some(&trusted));
    assert!(!v.ok);
    assert_eq!(v.first_bad_sequence, Some(3));

    // Drop an entry line.
    let mut dropped: Vec<&str> = lines.clone();
    dropped.remove(2);
    let v = chain::verify_jsonl(&dropped.join("\n"), Some(&trusted));
    assert_eq!(v.first_bad_sequence, Some(2));

    // A verifier pinning a different key rejects the anchors.
    let wrong = chain::AuditSigner::from_seed(&[1u8; 32]).unwrap();
    let v = chain::verify_jsonl(&text, Some(wrong.verifier()));
    assert!(!v.ok);
    assert_eq!(v.first_bad_anchor, Some(1));
}

// ── Migration & reset ──

#[tokio::test]
async fn migration_chains_existing_rows_in_order() {
    let pool = DbPool::open_in_memory().unwrap();
    pool.execute(|c| {
        crate::storage::migrations::run_migrations_up_to(c, 16)?;
        // Legacy rows, inserted out of timestamp order.
        for (id, ts) in [("b", "2025-01-02T00:00:00+00:00"), ("a", "2025-01-01T00:00:00+00:00"), ("c", "2025-01-03T00:00:00+00:00")] {
            c.execute(
                "INSERT INTO audit_events (id, timestamp, event_type, actor, resource, details) VALUES (?1, ?2, 'security.legacy', 'user', 'r', '')",
                [id, ts],
            )?;
        }
        Ok(())
    })
    .await
    .unwrap();
    let repo = Repository::new(pool.clone()).await.unwrap(); // applies v17
    let ids: Vec<String> = pool
        .execute(|c| {
            let mut s = c.prepare("SELECT id FROM audit_events ORDER BY sequence")?;
            let ids = s.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
            Ok(ids)
        })
        .await
        .unwrap();
    assert_eq!(ids, ["a", "b", "c"]);

    log_n(&repo, 1).await;
    let trail = AuditTrail::with_seed(pool, &SEED).unwrap();
    let v = trail.verify().await.unwrap();
    assert!(v.ok, "{v:?}");
    assert_eq!(v.entries_checked, 4);
    let e = raw(&repo, "DELETE FROM audit_events WHERE id = 'a'").await.unwrap_err();
    assert!(e.contains("append-only"));
}

#[tokio::test]
async fn reset_keeps_the_audit_log_and_its_key() {
    let repo = Repository::open_in_memory().await.unwrap();
    let settings = SettingsStore::plain(repo.pool().clone());
    let trail = AuditTrail::new(repo.pool().clone(), None, Some(settings.clone()));
    log_n(&repo, 3).await;
    trail.create_anchor().await.unwrap();
    assert_eq!(trail.status().await.unwrap().key_source, Some(KeySource::SettingsStorePlain));
    repo.set_config("some.setting", "x").await.unwrap();

    SecurityAudit::log_destructive_op(&repo, "reset_database", "test").await;
    repo.reset().await.unwrap();
    assert_eq!(repo.get_config("some.setting").await.unwrap(), None);

    // A fresh trail (new process) resolves the same key from settings.
    let after = AuditTrail::new(repo.pool().clone(), None, Some(settings));
    log_n(&repo, 1).await;
    let v = after.verify().await.unwrap();
    assert!(v.ok, "{v:?}");
    assert_eq!((v.entries_checked, v.anchors_verified), (5, 1));
}
