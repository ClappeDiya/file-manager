use super::*;
use crate::backup_engine::BackupManager;
use crate::storage::Repository;
use std::sync::Mutex;
use ufop_core::agent::mock::{MockConsole, MockRequest};
use ufop_core::backup::crypto::KdfParams;

struct Console {
    commands: Vec<Value>,
    completed: Vec<Value>,
    reports: Vec<Value>,
    policies: Vec<Value>,
    version: String,
}

fn console() -> (MockConsole, Arc<Mutex<Console>>) {
    let st = Arc::new(Mutex::new(Console { commands: vec![], completed: vec![], reports: vec![], policies: vec![], version: String::new() }));
    let s = st.clone();
    let m = MockConsole::start(move |r: &MockRequest| {
        let mut c = s.lock().unwrap();
        if r.path != "/api/agent/register" && r.bearer() != Some("ufop_agent_t1") {
            return (401, json!({ "error": "Unknown or revoked machine token" }));
        }
        match r.path.as_str() {
            "/api/agent/register" => (201, json!({ "machineId": "mch_1", "agentToken": "ufop_agent_t1", "organization": "Acme", "heartbeatSeconds": 60 })),
            "/api/agent/heartbeat" => {
                let changed = r.body["policySetVersion"] != json!(c.version);
                (200, json!({ "status": "active", "commands": std::mem::take(&mut c.commands), "policySetVersion": c.version, "policiesChanged": changed, "heartbeatSeconds": 60 }))
            }
            "/api/agent/policies" => (
                200,
                json!({ "policySetVersion": c.version, "policies": c.policies, "credentials": [{ "credentialId": "central:crd_pw", "name": "pw", "secret": "Corr3ct-Horse-Battery", "updatedAt": "" }] }),
            ),
            "/api/agent/policies/applied" => (200, json!({ "ok": true })),
            "/api/agent/report" => {
                c.reports.push(r.body.clone());
                (201, json!({ "id": "rpt" }))
            }
            p if p.starts_with("/api/agent/commands/") => {
                c.completed.push(json!({ "id": &p[20..], "body": r.body }));
                (200, json!({ "ok": true }))
            }
            _ => (404, json!({ "error": "not found" })),
        }
    })
    .unwrap();
    (m, st)
}

async fn idle(m: &BackupManager) {
    for _ in 0..400 {
        if m.status().await.tasks.is_empty() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("backup did not finish");
}

#[tokio::test]
async fn managed_job_from_policy_runs_and_reports() {
    let t = tempfile::tempdir().unwrap();
    let (src, dst) = (t.path().join("src"), t.path().join("dst"));
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("a.txt"), b"alpha").unwrap();
    let repo = Repository::open_in_memory().await.unwrap();
    let creds = Arc::new(CredentialStore::new());
    let mgr = BackupManager::with_kdf(repo.pool().clone(), creds.clone(), None, None, t.path().join("dbs"), KdfParams::insecure_fast());
    let settings = SettingsStore::plain(repo.pool().clone());
    mgr.set_settings_store(settings.clone());
    let agent = init(mgr.clone(), settings.clone(), creds.clone(), "desktop").await.unwrap();
    let (m, c) = console();

    let st = agent.register(&m.url, "ufop_reg_x", vec![]).await.unwrap();
    assert!(st.registered);
    // The token is not in the saved configuration.
    let saved = settings.get(CONFIG_KEY).await.unwrap().unwrap();
    assert!(!saved.contains("ufop_agent_t1") && saved.contains("mch_1"));
    assert!(m.hits("/api/agent/register")[0].body["info"]["agentKind"] == "desktop");

    {
        let mut c = c.lock().unwrap();
        c.version = "fpl_1:1".into();
        c.policies = vec![json!({ "id": "fpl_1", "name": "Docs", "version": 1, "job": {
            "name": "Fleet docs", "sources": [src.to_string_lossy()], "encrypted": true, "passphraseCredentialId": "central:crd_pw",
            "destination": { "type": "local", "path": dst.to_string_lossy() },
            "options": { "block_size": 4096, "volume_size": 65536, "quota_disable": true },
            "schedule": { "enabled": true, "start_time": "02:00", "interval_minutes": 720 }
        }})];
    }
    agent.cycle().await.unwrap();
    let job = mgr.get_job("policy-fpl_1").await.unwrap();
    assert_eq!(job.name, "Fleet docs");
    assert!(job.description.contains("Managed"));
    // The passphrase is a reference to the central credential, resolved at run time.
    assert_eq!(mgr.passphrase("policy-fpl_1").await.unwrap().as_deref(), Some("Corr3ct-Horse-Battery"));
    let rows: Vec<String> = repo
        .pool()
        .execute(|conn| {
            let mut s = conn.prepare("SELECT value FROM config UNION ALL SELECT config_json FROM backup_jobs")?;
            let v = s.query_map([], |r| r.get(0))?.collect::<Result<Vec<String>, _>>()?;
            Ok(v)
        })
        .await
        .unwrap();
    // No keychain and an unencrypted settings store here: the central
    // credential stays in memory, never in the database.
    assert!(!rows.iter().any(|r| r.contains("Corr3ct-Horse-Battery")), "no plaintext secret on disk");
    assert_eq!(agent.host().jobs().await[0].managed_by_policy.as_deref(), Some("fpl_1"));
    assert_eq!(agent.host().jobs().await[0].schedule_interval_minutes, Some(720));

    // run_backup and an rpc call from the console.
    c.lock().unwrap().commands = vec![
        json!({ "id": "cmd_run", "kind": "run_backup", "payload": { "jobId": "policy-fpl_1" } }),
        json!({ "id": "cmd_rpc", "kind": "rpc", "payload": { "command": "backup_get_job", "args": { "id": "policy-fpl_1" } } }),
        json!({ "id": "cmd_no", "kind": "rpc", "payload": { "command": "backup_get_secret_settings", "args": {} } }),
    ];
    agent.cycle().await.unwrap();
    idle(&mgr).await;
    {
        let c = c.lock().unwrap();
        assert_eq!(c.completed[0]["body"]["ok"], true);
        assert!(c.completed[0]["body"]["result"]["taskId"].is_string());
        assert_eq!(c.completed[1]["body"]["result"]["name"], "Fleet docs");
        assert_eq!(c.completed[2]["body"]["ok"], false);
    }
    assert_eq!(agent.status().pending_reports, 1);
    agent.cycle().await.unwrap();
    let rep = c.lock().unwrap().reports[0].clone();
    assert_eq!(rep["backup_id"], "policy-fpl_1");
    assert_eq!(rep["operation"], "backup");
    assert_eq!(rep["result"], "success", "{rep}");
    assert_eq!(rep["stats"]["version_count"], 1);
    assert_eq!(rep["stats"]["schedule_interval_minutes"], 720);
    assert!(rep["stats"]["backend_size"].as_u64().unwrap() > 0);
    assert_eq!(rep["data"]["files_examined"], 1);
    assert!(rep["data"].get("log").is_none());

    // Policy withdrawn: the managed job goes, the backup data stays.
    {
        let mut c = c.lock().unwrap();
        c.version = String::new();
        c.policies.clear();
    }
    agent.cycle().await.unwrap();
    assert!(mgr.get_job("policy-fpl_1").await.is_err());
    assert!(std::fs::read_dir(&dst).unwrap().count() > 0);

    // Server-mode RPC surface.
    let v = dispatch(&agent, "fleet_agent_status", &json!({})).await.unwrap();
    assert_eq!(v["machine_id"], "mch_1");
    assert!(dispatch(&agent, "fleet_agent_nope", &json!({})).await.is_err());
    let v = dispatch(&agent, "fleet_agent_set_enabled", &json!({ "enabled": false })).await.unwrap();
    assert_eq!(v["enabled"], false);
}

#[tokio::test]
async fn central_placeholder_needs_a_delivered_credential() {
    let t = tempfile::tempdir().unwrap();
    let repo = Repository::open_in_memory().await.unwrap();
    let mgr = BackupManager::with_kdf(repo.pool().clone(), Arc::new(CredentialStore::new()), None, None, t.path().join("dbs"), KdfParams::insecure_fast());
    mgr.set_settings_store(SettingsStore::plain(repo.pool().clone()));
    let src = t.path().join("s");
    std::fs::create_dir_all(&src).unwrap();
    let job = BackupJob {
        name: "J".into(),
        sources: vec![src.to_string_lossy().into()],
        destination: crate::backup_engine::BackupDestination::Local { path: t.path().join("d").to_string_lossy().into(), alternate_paths: vec![], marker_file: None },
        ..Default::default()
    };
    let j = mgr.save_job(job, Some("$central:crd_missing".into())).await.unwrap();
    let e = mgr.passphrase(&j.id).await.unwrap_err();
    assert!(e.user_message().contains("has not been delivered"), "{}", e.user_message());
    mgr.set_destination_secret("central:crd_missing", "pw").await.unwrap();
    assert_eq!(mgr.passphrase(&j.id).await.unwrap().as_deref(), Some("pw"));
}

#[tokio::test]
async fn local_limits_confine_policies_and_remote_calls() {
    let t = tempfile::tempdir().unwrap();
    let allowed = t.path().join("allowed");
    let (src, dst) = (allowed.join("src"), allowed.join("dst"));
    std::fs::create_dir_all(&src).unwrap();
    let outside = t.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    let repo = Repository::open_in_memory().await.unwrap();
    let creds = Arc::new(CredentialStore::new());
    let mgr = BackupManager::with_kdf(repo.pool().clone(), creds.clone(), None, None, t.path().join("dbs"), KdfParams::insecure_fast());
    let settings = SettingsStore::plain(repo.pool().clone());
    mgr.set_settings_store(settings.clone());
    let agent = init(mgr.clone(), settings.clone(), creds.clone(), "desktop").await.unwrap();
    let (m, c) = console();
    agent.register(&m.url, "ufop_reg_x", vec![]).await.unwrap();
    assert_eq!(agent.host().limits().await, AgentLimits::default());
    agent
        .host()
        .set_limits(&AgentLimits { roots: vec![allowed.to_string_lossy().into()], allow_scripts: false, ..Default::default() })
        .await
        .unwrap();
    assert!(agent.host().set_limits(&AgentLimits { roots: vec!["relative".into()], ..Default::default() }).await.is_err());

    let policy = |id: &str, source: &std::path::Path, scripts: Value| {
        json!({ "id": id, "name": id, "version": 1, "job": {
            "name": id, "sources": [source.to_string_lossy()], "encrypted": false,
            "destination": { "type": "local", "path": dst.to_string_lossy() }, "scripts": scripts
        }})
    };
    {
        let mut c = c.lock().unwrap();
        c.version = "v1".into();
        c.policies = vec![policy("fpl_in", &src, json!({})), policy("fpl_out", &outside, json!({})), policy("fpl_sh", &src, json!({ "run_before": "touch /tmp/pwned" }))];
    }
    agent.cycle().await.unwrap();
    let st = agent.status();
    let by_id = |id: &str| st.managed_policies.iter().find(|p| p.policy_id == id).cloned().unwrap();
    assert!(by_id("fpl_in").error.is_none());
    assert!(by_id("fpl_out").error.unwrap().contains("outside the folders"));
    assert!(by_id("fpl_sh").error.unwrap().contains("scripts"));
    assert!(mgr.get_job("policy-fpl_out").await.is_err() && mgr.get_job("policy-fpl_sh").await.is_err());

    // Remote calls: a restore outside the roots is refused, inside is accepted.
    let restore = |target: &std::path::Path| json!({ "command": "backup_start_operation", "args": { "id": "policy-fpl_in", "operation": { "type": "restore", "options": { "version": 0, "target": target.to_string_lossy() } } } });
    c.lock().unwrap().commands = vec![json!({ "id": "cmd_out", "kind": "rpc", "payload": restore(&outside) })];
    agent.cycle().await.unwrap();
    let done = c.lock().unwrap().completed.last().cloned().unwrap();
    assert_eq!(done["body"]["ok"], false);
    assert!(done["body"]["result"].to_string().contains("outside the folders"), "{done}");

    // Read-only: the console can no longer change jobs.
    agent.host().set_limits(&AgentLimits { read_only: true, ..Default::default() }).await.unwrap();
    c.lock().unwrap().commands = vec![
        json!({ "id": "cmd_del", "kind": "rpc", "payload": { "command": "backup_delete_job", "args": { "id": "policy-fpl_in", "deleteLocalDb": false, "deleteRemote": false } } }),
        json!({ "id": "cmd_get", "kind": "rpc", "payload": { "command": "backup_get_job", "args": { "id": "policy-fpl_in" } } }),
    ];
    agent.cycle().await.unwrap();
    let done = c.lock().unwrap().completed.clone();
    let n = done.len();
    assert_eq!(done[n - 2]["body"]["ok"], false);
    assert!(done[n - 2]["body"]["result"].to_string().contains("read-only"));
    assert_eq!(done[n - 1]["body"]["ok"], true);
    assert!(mgr.get_job("policy-fpl_in").await.is_ok());

    // Unreadable limits fail closed.
    settings.set(LIMITS_KEY, "{not json").await.unwrap();
    let l = agent.host().limits().await;
    assert!(l.read_only && !l.allow_scripts && l.check_path(&src.to_string_lossy()).is_err());
}
