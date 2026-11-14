use super::mock::{MockConsole, MockRequest};
use super::policy::*;
use super::*;
use crate::error::AppError;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct HostState {
    cfg: AgentConfig,
    token: Option<String>,
    creds: BTreeMap<String, String>,
    jobs: BTreeMap<String, Value>,
    rpc_calls: Vec<(String, Value)>,
    runs: Vec<String>,
    /// Size of the diagnostics bundle the host builds (`None`: unsupported).
    diag_size: Option<usize>,
    /// Managed settings applied by the agent, in order (`None` = released).
    settings: Vec<Option<ManagedSettings>>,
    fail_settings: bool,
}

#[derive(Clone, Default)]
struct FakeHost(Arc<Mutex<HostState>>);

impl FakeHost {
    fn st(&self) -> std::sync::MutexGuard<'_, HostState> {
        self.0.lock().unwrap()
    }
}

impl AgentHost for FakeHost {
    fn client_info(&self) -> ClientInfo {
        ClientInfo { hostname: "box1".into(), os: "linux".into(), client_version: "9.9".into(), agent_kind: "cli".into(), ..Default::default() }
    }
    fn load(&self) -> BoxFuture<'_, Result<(AgentConfig, Option<String>), AppError>> {
        Box::pin(async move {
            let s = self.st();
            Ok((s.cfg.clone(), s.token.clone()))
        })
    }
    fn save_config(&self, cfg: AgentConfig) -> BoxFuture<'_, Result<(), AppError>> {
        Box::pin(async move {
            self.st().cfg = cfg;
            Ok(())
        })
    }
    fn save_token(&self, token: Option<String>) -> BoxFuture<'_, Result<(), AppError>> {
        Box::pin(async move {
            self.st().token = token;
            Ok(())
        })
    }
    fn jobs(&self) -> BoxFuture<'_, Vec<JobSummary>> {
        Box::pin(async move {
            self.st()
                .jobs
                .keys()
                .map(|id| JobSummary { id: id.clone(), name: id.clone(), managed_by_policy: policy_of_job(id).map(String::from), ..Default::default() })
                .collect()
        })
    }
    fn rpc(&self, command: String, args: Value) -> BoxFuture<'_, Result<Value, AppError>> {
        Box::pin(async move {
            self.st().rpc_calls.push((command.clone(), args));
            if command == "backup_get_job" {
                return Err(AppError::not_found("Backup job x not found"));
            }
            Ok(json!([{ "id": "j1" }]))
        })
    }
    fn run_backup(&self, job_id: String) -> BoxFuture<'_, Result<Value, AppError>> {
        Box::pin(async move {
            self.st().runs.push(job_id);
            Ok(json!({ "taskId": "t1" }))
        })
    }
    fn store_credential(&self, id: String, secret: Option<String>) -> BoxFuture<'_, Result<(), AppError>> {
        Box::pin(async move {
            match secret {
                Some(s) => self.st().creds.insert(id, s),
                None => self.st().creds.remove(&id),
            };
            Ok(())
        })
    }
    fn apply_policy(&self, p: FleetPolicy) -> BoxFuture<'_, Result<(), AppError>> {
        Box::pin(async move {
            let t = prepare_template(&p.job)?;
            if t.fields.get("sources").is_none() {
                return Err(AppError::validation("Add at least one source folder"));
            }
            let mut job = Value::Object(t.fields);
            job["passphrase"] = json!(t.passphrase);
            self.st().jobs.insert(managed_job_id(&p.id)?, job);
            Ok(())
        })
    }
    fn remove_policy(&self, id: String) -> BoxFuture<'_, Result<(), AppError>> {
        Box::pin(async move {
            self.st().jobs.remove(&format!("policy-{id}"));
            Ok(())
        })
    }
    fn managed_policies(&self) -> BoxFuture<'_, Vec<String>> {
        Box::pin(async move { self.st().jobs.keys().filter_map(|k| policy_of_job(k).map(String::from)).collect() })
    }
    fn apply_managed_settings(&self, settings: Option<ManagedSettings>) -> BoxFuture<'_, Result<(), AppError>> {
        Box::pin(async move {
            if self.st().fail_settings {
                return Err(AppError::validation("proxy unreachable"));
            }
            self.st().settings.push(settings);
            Ok(())
        })
    }
    fn diagnostics(&self) -> BoxFuture<'_, Result<DiagnosticsUpload, AppError>> {
        Box::pin(async move {
            match self.st().diag_size {
                Some(n) => Ok(DiagnosticsUpload { file_name: "diag.zip".into(), summary: json!({ "os": "linux" }), bytes: vec![b'P'; n] }),
                None => Err(AppError::state("Diagnostics are not available on this client", "")),
            }
        })
    }
}

/// A small stateful console: one org, tokens, a command queue, policies.
#[derive(Default)]
struct Console {
    token: String,
    revoked: bool,
    commands: Vec<Value>,
    completed: Vec<Value>,
    policies: Vec<Value>,
    version: String,
    reports: Vec<Value>,
    applied: Value,
    claim_polls: u32,
    rotations: u32,
    diagnostics: Vec<Value>,
    managed_settings: Value,
    secret: Option<String>,
    cred_version: u64,
}

fn console() -> (MockConsole, Arc<Mutex<Console>>) {
    let st = Arc::new(Mutex::new(Console { token: "ufop_agent_first".into(), ..Default::default() }));
    let s = st.clone();
    let m = MockConsole::start(move |r: &MockRequest| {
        let mut c = s.lock().unwrap();
        let authed = r.bearer() == Some(c.token.as_str()) && !c.revoked;
        let path = r.path.as_str();
        match path {
            "/api/agent/register" => {
                if r.body["registrationToken"] != "ufop_reg_good" {
                    return (401, json!({ "error": "The registration token is not valid" }));
                }
                (201, json!({ "machineId": "mch_1", "agentToken": c.token, "organization": "Acme", "heartbeatSeconds": 20 }))
            }
            "/api/agent/connect/token" => {
                if r.body["code"] != "ufop_connect_ok" || r.body["codeVerifier"] != "verifier" || r.body["redirectUri"] != "http://127.0.0.1:5/callback" {
                    return (401, json!({ "error": "The sign-in code is not valid or has expired; sign in again" }));
                }
                (201, json!({ "machineId": "mch_3", "agentToken": c.token, "organization": "Acme", "heartbeatSeconds": 45 }))
            }
            "/api/agent/claim" => (201, json!({ "claimId": "clm_1", "code": "ABCD-EFGH", "secret": "ufop_claim_s", "expiresAt": "2099-01-01T00:00:00Z" })),
            "/api/agent/claim/poll" => {
                c.claim_polls += 1;
                if c.claim_polls < 2 {
                    (200, json!({ "status": "pending" }))
                } else {
                    (200, json!({ "status": "collected", "machineId": "mch_2", "agentToken": c.token }))
                }
            }
            _ if !authed => (401, json!({ "error": "Unknown or revoked machine token", "code": "unauthorized" })),
            "/api/agent/heartbeat" => {
                let cmds = std::mem::take(&mut c.commands);
                let changed = r.body["policySetVersion"] != json!(c.version);
                (200, json!({ "machineId": "mch_1", "status": "active", "commands": cmds, "policySetVersion": c.version, "policiesChanged": changed, "heartbeatSeconds": 30 }))
            }
            "/api/agent/policies" => (
                200,
                json!({ "policySetVersion": c.version, "policies": c.policies, "credentials": [{ "credentialId": "central:crd_1", "name": "pw", "secret": c.secret.clone().unwrap_or_else(|| "s3cret".into()), "updatedAt": "", "version": c.cred_version }], "managedSettings": c.managed_settings }),
            ),
            "/api/agent/policies/applied" => {
                c.applied = r.body.clone();
                (200, json!({ "ok": true }))
            }
            "/api/agent/report" => {
                c.reports.push(r.body.clone());
                (201, json!({ "id": "rpt_1" }))
            }
            "/api/agent/diagnostics" => {
                c.diagnostics.push(r.body.clone());
                (201, json!({ "id": format!("dgn_{}", c.diagnostics.len()) }))
            }
            "/api/agent/rotate" => {
                c.rotations += 1;
                c.token = format!("ufop_agent_rotated{}", c.rotations);
                (200, json!({ "agentToken": c.token }))
            }
            "/api/agent/unregister" => {
                c.revoked = true;
                (200, json!({ "ok": true }))
            }
            p if p.starts_with("/api/agent/commands/") => {
                c.completed.push(json!({ "id": &p[20..], "body": r.body, "auth": r.bearer() }));
                (200, json!({ "ok": true }))
            }
            _ => (404, json!({ "error": "not found" })),
        }
    })
    .unwrap();
    (m, st)
}

async fn agent(host: &FakeHost) -> Arc<Agent<FakeHost>> {
    Agent::load(host.clone(), reqwest::Client::new()).await.unwrap()
}

fn policy_json(id: &str, version: u64) -> Value {
    json!({ "id": id, "name": "Docs", "version": version, "job": {
        "name": "Docs", "sources": ["/data"], "encrypted": true, "passphraseCredentialId": "central:crd_1",
        "destination": { "type": "cloud", "config": { "provider": "s3" }, "credential_id": "central:crd_1" },
        "notifications": [{ "password": "central:crd_1" }]
    }})
}

#[tokio::test]
async fn register_heartbeat_commands_and_policies() {
    let (m, c) = console();
    let host = FakeHost::default();
    let a = agent(&host).await;
    let bad = a.register(&m.url, "ufop_reg_wrong", vec![]).await.unwrap_err();
    assert!(bad.user_message().contains("not valid"), "{}", bad.user_message());
    assert!(a.register(&m.url, "nope", vec![]).await.is_err());

    let st = a.register(&format!("{}/", m.url), "ufop_reg_good", vec!["lab".into()]).await.unwrap();
    assert!(st.registered && st.enabled);
    assert_eq!((st.machine_id.as_str(), st.organization.as_str(), st.heartbeat_seconds), ("mch_1", "Acme", 20));
    assert_eq!(host.st().token.as_deref(), Some("ufop_agent_first"));
    assert_eq!(host.st().cfg.console_url, m.url, "trailing slash trimmed");
    let reg = m.hits("/api/agent/register").pop().unwrap();
    assert_eq!(reg.body["info"]["hostname"], "box1");
    assert_eq!(reg.body["tags"], json!(["lab"]));
    assert!(a.register(&m.url, "ufop_reg_good", vec![]).await.is_err(), "already registered");

    {
        let mut c = c.lock().unwrap();
        c.version = "fpl_a:1".into();
        c.policies = vec![policy_json("fpl_a", 1)];
        c.commands = vec![
            json!({ "id": "cmd_1", "kind": "rpc", "payload": { "command": "backup_list_jobs" } }),
            json!({ "id": "cmd_2", "kind": "rpc", "payload": { "command": "backup_export_config", "args": { "id": "x", "includePassphrase": true } } }),
            json!({ "id": "cmd_3", "kind": "run_backup", "payload": { "jobId": "policy-fpl_a" } }),
            json!({ "id": "cmd_4", "kind": "rpc", "payload": { "command": "backup_get_job", "args": { "id": "x" } } }),
            json!({ "id": "cmd_5", "kind": "bogus", "payload": {} }),
        ];
    }
    a.cycle().await.unwrap();
    let hb = &m.hits("/api/agent/heartbeat")[0];
    assert_eq!(hb.bearer(), Some("ufop_agent_first"));
    assert_eq!(hb.body["info"]["agentKind"], "cli");
    let completed = c.lock().unwrap().completed.clone();
    assert_eq!(completed.len(), 5);
    assert_eq!(completed[0]["body"], json!({ "ok": true, "result": [{ "id": "j1" }] }));
    assert_eq!(completed[1]["body"]["ok"], false, "not allow-listed");
    assert!(completed[1]["body"]["result"]["error"].as_str().unwrap().contains("may not be called remotely"));
    assert_eq!(completed[2]["body"]["result"]["taskId"], "t1");
    assert_eq!(completed[3]["body"]["ok"], false);
    assert_eq!(completed[4]["body"]["ok"], false);
    {
        let h = host.st();
        assert_eq!(h.rpc_calls, vec![("backup_list_jobs".to_string(), json!({})), ("backup_get_job".to_string(), json!({ "id": "x" }))]);
        assert_eq!(h.runs, vec!["policy-fpl_a"]);
        // Policy applied: credential in the store, references converted.
        assert_eq!(h.creds.get("central:crd_1").map(String::as_str), Some("s3cret"));
        let job = &h.jobs["policy-fpl_a"];
        assert_eq!(job["destination"]["credential_id"], "central:crd_1");
        assert_eq!(job["notifications"][0]["password"], "$central:crd_1");
        assert_eq!(job["passphrase"], "$central:crd_1");
        assert!(!job.to_string().contains("s3cret"), "no secret in the job");
        assert_eq!(h.cfg.policy_set_version, "fpl_a:1");
        assert_eq!(h.cfg.heartbeat_seconds, 30);
    }
    assert_eq!(c.lock().unwrap().applied, json!({ "applied": { "fpl_a": 1 }, "errors": {} }));

    // Unchanged set: no policy fetch. A broken template is reported, a removed policy's job deleted.
    a.cycle().await.unwrap();
    assert_eq!(m.hits("/api/agent/policies").len(), 1);
    {
        let mut c = c.lock().unwrap();
        c.version = "fpl_b:1".into();
        c.policies = vec![json!({ "id": "fpl_b", "name": "B", "version": 1, "job": { "name": "B" } })];
    }
    a.cycle().await.unwrap();
    let st = a.status();
    assert!(host.st().jobs.is_empty(), "policy-fpl_a removed");
    assert_eq!(st.managed_policies.len(), 1);
    assert!(st.managed_policies[0].error.as_deref().unwrap().contains("source"));
    assert!(c.lock().unwrap().applied["errors"]["fpl_b"].is_string());

    // apply_policies command forces a fetch and returns the sync result.
    c.lock().unwrap().commands = vec![json!({ "id": "cmd_6", "kind": "apply_policies", "payload": {} })];
    a.cycle().await.unwrap();
    let last = c.lock().unwrap().completed.last().cloned().unwrap();
    assert_eq!(last["body"]["result"]["policy_set_version"], "fpl_b:1");
}

#[tokio::test]
async fn rotate_command_completes_with_new_token() {
    let (m, c) = console();
    let host = FakeHost::default();
    let a = agent(&host).await;
    a.register(&m.url, "ufop_reg_good", vec![]).await.unwrap();
    c.lock().unwrap().commands = vec![json!({ "id": "cmd_r", "kind": "rotate_token", "payload": {} }), json!({ "id": "cmd_n", "kind": "rpc", "payload": { "command": "backup_status" } })];
    a.cycle().await.unwrap();
    let done = c.lock().unwrap().completed.clone();
    assert_eq!(done[0]["auth"], "ufop_agent_rotated1");
    assert_eq!(done[0]["body"]["ok"], true);
    assert_eq!(done[1]["auth"], "ufop_agent_rotated1");
    assert_eq!(host.st().token.as_deref(), Some("ufop_agent_rotated1"));
    a.cycle().await.unwrap();
    assert_eq!(m.hits("/api/agent/heartbeat").last().unwrap().bearer(), Some("ufop_agent_rotated1"));
}

#[tokio::test]
async fn browser_sign_in_builds_the_url_and_redeems_the_code() {
    let (m, _c) = console();
    let host = FakeHost::default();
    let a = agent(&host).await;
    let url = reqwest::Url::parse(&a.connect_url(&m.url, "http://127.0.0.1:5/callback", "st", "ch").unwrap()).unwrap();
    assert_eq!(url.path(), "/fleet/connect");
    let q: BTreeMap<String, String> = url.query_pairs().into_owned().collect();
    assert_eq!(q["redirect_uri"], "http://127.0.0.1:5/callback");
    assert_eq!(q["state"], "st");
    assert_eq!(q["code_challenge_method"], "S256");
    assert_eq!(q["hostname"], "box1");

    let err = a.register_with_code(&m.url, "ufop_connect_bad", "verifier", "http://127.0.0.1:5/callback", vec![]).await.unwrap_err();
    assert!(err.user_message().contains("sign in again"), "{}", err.user_message());
    let st = a.register_with_code(&m.url, "ufop_connect_ok", "verifier", "http://127.0.0.1:5/callback", vec!["dev".into()]).await.unwrap();
    assert!(st.registered);
    assert_eq!(st.machine_id, "mch_3");
    assert_eq!(st.registered_via, "sso");
    assert_eq!(host.st().token.as_deref(), Some("ufop_agent_first"));
    // Already registered: no second sign-in.
    assert!(a.connect_url(&m.url, "http://127.0.0.1:5/callback", "st", "ch").is_err());
}

#[tokio::test]
async fn managed_settings_apply_on_change_report_errors_and_release() {
    let (m, c) = console();
    let host = FakeHost::default();
    let a = agent(&host).await;
    a.register(&m.url, "ufop_reg_good", vec![]).await.unwrap();
    let set = |v: Value, version: &str| {
        let mut g = c.lock().unwrap();
        g.managed_settings = v;
        g.version = version.into();
    };
    set(json!({ "version": 2, "updates": { "channel": "beta" }, "ai": { "enabled": false } }), "s2");
    a.cycle().await.unwrap();
    let applied = host.st().settings.clone();
    assert_eq!(applied.len(), 1);
    let s = applied[0].clone().unwrap();
    assert_eq!(s.updates.unwrap().channel.as_deref(), Some("beta"));
    assert_eq!(c.lock().unwrap().applied["applied"][MANAGED_SETTINGS_KEY], 2);
    assert!(a.status().managed_policies.is_empty(), "settings are not listed as a policy");

    // Same version again: not re-applied.
    set(json!({ "version": 2, "updates": { "channel": "beta" } }), "s2b");
    a.cycle().await.unwrap();
    assert_eq!(host.st().settings.len(), 1);

    // Invalid or failing settings are reported, the old version stays.
    set(json!({ "version": 3, "proxy": { "mode": "manual" } }), "s3");
    a.cycle().await.unwrap();
    assert!(c.lock().unwrap().applied["errors"][MANAGED_SETTINGS_KEY].as_str().unwrap().contains("URL"));
    assert_eq!(a.config().managed_settings_version, 2);
    host.st().fail_settings = true;
    set(json!({ "version": 4, "proxy": { "mode": "none" } }), "s4");
    a.cycle().await.unwrap();
    assert!(c.lock().unwrap().applied["errors"][MANAGED_SETTINGS_KEY].as_str().unwrap().contains("unreachable"));
    host.st().fail_settings = false;
    a.cycle().await.unwrap();
    assert_eq!(a.config().managed_settings_version, 2, "no policy-set change, no retry until the next change");

    // The console stops managing: settings are released.
    set(Value::Null, "s5");
    a.cycle().await.unwrap();
    assert_eq!(host.st().settings.last().unwrap(), &None);
    assert_eq!(a.config().managed_settings_version, 0);

    set(json!({ "version": 6, "ai": { "allowCloud": false } }), "s6");
    a.cycle().await.unwrap();
    // A restarted agent applies the current settings again once.
    let before = host.st().settings.len();
    let restarted = agent(&host).await;
    restarted.cycle().await.unwrap();
    assert_eq!(host.st().settings.len(), before + 1);
    restarted.cycle().await.unwrap();
    assert_eq!(host.st().settings.len(), before + 1);
    a.unregister(false).await.unwrap();
    assert_eq!(host.st().settings.last().unwrap(), &None, "unregistering releases them");
}

#[tokio::test]
async fn rotated_credentials_are_stored_and_their_version_reported() {
    let (m, c) = console();
    let host = FakeHost::default();
    let a = agent(&host).await;
    a.register(&m.url, "ufop_reg_good", vec![]).await.unwrap();
    {
        let mut g = c.lock().unwrap();
        g.version = "cred:crd_1:1".into();
        g.cred_version = 1;
    }
    a.cycle().await.unwrap();
    assert_eq!(host.st().creds["central:crd_1"], "s3cret");
    assert_eq!(c.lock().unwrap().applied["applied"]["credential:crd_1"], 1);
    {
        let mut g = c.lock().unwrap();
        g.version = "cred:crd_1:2".into();
        g.cred_version = 2;
        g.secret = Some("rotated".into());
    }
    a.cycle().await.unwrap();
    assert_eq!(host.st().creds["central:crd_1"], "rotated");
    assert_eq!(c.lock().unwrap().applied["applied"]["credential:crd_1"], 2);
    assert!(a.status().managed_policies.is_empty(), "credential versions are not listed as policies");
}

#[tokio::test]
async fn diagnostics_upload_on_demand_and_by_command() {
    let (m, c) = console();
    let host = FakeHost::default();
    let a = agent(&host).await;
    assert!(a.send_diagnostics().await.unwrap_err().user_message().contains("not registered"));
    a.register(&m.url, "ufop_reg_good", vec![]).await.unwrap();

    // Hosts without a builder fail the command instead of crashing the cycle.
    c.lock().unwrap().commands = vec![json!({ "id": "cmd_d0", "kind": "collect_diagnostics", "payload": {} })];
    a.cycle().await.unwrap();
    assert_eq!(c.lock().unwrap().completed[0]["body"]["ok"], false);

    host.st().diag_size = Some(4);
    assert_eq!(a.send_diagnostics().await.unwrap(), "dgn_1");
    let sent = c.lock().unwrap().diagnostics[0].clone();
    assert_eq!(sent["fileName"], "diag.zip");
    assert_eq!(sent["bundleBase64"], "UFBQUA==");
    assert_eq!(sent["summary"]["os"], "linux");

    c.lock().unwrap().commands = vec![json!({ "id": "cmd_d1", "kind": "collect_diagnostics", "payload": {} })];
    a.cycle().await.unwrap();
    let done = c.lock().unwrap().completed.last().cloned().unwrap();
    assert_eq!(done["body"]["ok"], true);
    assert_eq!(done["body"]["result"]["diagnosticsId"], "dgn_2");

    // Oversized bundles are refused locally, never uploaded.
    host.st().diag_size = Some(MAX_DIAGNOSTICS_BYTES + 1);
    assert!(a.send_diagnostics().await.is_err());
    assert_eq!(c.lock().unwrap().diagnostics.len(), 2);
}

fn report(i: u64) -> RunReport {
    RunReport { backup_id: "j1".into(), name: "Docs".into(), operation: "backup".into(), result: "success".into(), duration_ms: i, ..Default::default() }
}

#[tokio::test]
async fn reports_queue_while_offline_and_are_bounded() {
    let (m, c) = console();
    let host = FakeHost::default();
    let a = agent(&host).await;
    a.enqueue_report(&report(0));
    assert_eq!(a.status().pending_reports, 0, "not registered: nothing queued");
    a.register(&m.url, "ufop_reg_good", vec![]).await.unwrap();

    // Point at a closed port: the cycle fails and reports stay queued.
    let dead = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let dead_url = format!("http://{}", dead.local_addr().unwrap());
    drop(dead);
    let real = a.config().console_url;
    a.test_set_url(&dead_url);
    for i in 0..(MAX_PENDING_REPORTS as u64 + 5) {
        a.enqueue_report(&report(i));
    }
    let e = a.cycle().await.unwrap_err();
    assert!(e.user_message().contains("could not be reached"), "{}", e.user_message());
    let st = a.status();
    assert_eq!((st.pending_reports, st.dropped_reports), (MAX_PENDING_REPORTS, 5));
    assert!(st.last_error.is_some());
    assert_eq!(host.st().cfg.pending_reports.len(), MAX_PENDING_REPORTS, "queue persisted");

    a.test_set_url(&real);
    a.cycle().await.unwrap();
    let got = c.lock().unwrap().reports.clone();
    assert_eq!(got.len(), MAX_PENDING_REPORTS);
    assert_eq!(got[0]["duration_ms"], 5, "oldest dropped, order kept");
    assert_eq!(got[0]["backup_id"], "j1");
    assert_eq!(a.status().pending_reports, 0);
    assert!(a.status().last_error.is_none());
}

#[tokio::test]
async fn claim_flow_and_revocation_and_unregister() {
    let (m, c) = console();
    let host = FakeHost::default();
    let a = agent(&host).await;
    assert_eq!(a.poll_claim().await.unwrap().state, "none");
    let cl = a.start_claim(&m.url).await.unwrap();
    assert_eq!(cl.code, "ABCD-EFGH");
    assert_eq!(a.status().claim.unwrap().code, "ABCD-EFGH");
    assert_eq!(a.poll_claim().await.unwrap().state, "pending");
    let r = a.poll_claim().await.unwrap();
    assert_eq!(r.state, "registered");
    assert!(r.status.registered && r.status.claim.is_none());
    assert_eq!((r.status.machine_id.as_str(), r.status.registered_via.as_str()), ("mch_2", "claim"));
    let poll = &m.hits("/api/agent/claim/poll")[0];
    assert_eq!(poll.body, json!({ "claimId": "clm_1", "secret": "ufop_claim_s" }));

    // Policies give a managed job and a credential; unregister removes both.
    {
        let mut c = c.lock().unwrap();
        c.version = "fpl_a:1".into();
        c.policies = vec![policy_json("fpl_a", 1)];
    }
    a.cycle().await.unwrap();
    assert_eq!(host.st().jobs.len(), 1);
    let st = a.unregister(false).await.unwrap();
    assert!(!st.registered);
    {
        let h = host.st();
        assert!(h.jobs.is_empty() && h.creds.is_empty() && h.token.is_none());
        assert!(h.cfg.auto_register_declined);
    }
    let env = |k: &str| match k {
        ENV_URL => Some(m.url.clone()),
        ENV_TOKEN => Some("ufop_reg_good".to_string()),
        ENV_TAGS => Some("Linux, lab".to_string()),
        _ => None,
    };
    assert!(!a.auto_register(env).await.unwrap(), "declined after an explicit unregister");

    // Fresh machine: auto-registration from the environment, then revocation.
    let host2 = FakeHost::default();
    let b = agent(&host2).await;
    assert!(!b.auto_register(|_| None).await.unwrap());
    c.lock().unwrap().revoked = false;
    assert!(b.auto_register(env).await.unwrap());
    assert_eq!(b.status().tags, vec!["lab", "linux"]);
    c.lock().unwrap().revoked = true;
    let e = b.cycle().await.unwrap_err();
    assert!(e.user_message().contains("refused"), "{}", e.user_message());
    assert_eq!(b.status().machine_status, "revoked");
    // Unregister works even though the console no longer knows the token.
    assert!(!b.unregister(false).await.unwrap().registered);
}

#[tokio::test]
async fn disabled_agent_does_nothing() {
    let (m, _c) = console();
    let host = FakeHost::default();
    let a = agent(&host).await;
    a.register(&m.url, "ufop_reg_good", vec![]).await.unwrap();
    assert!(!a.set_enabled(false).await.enabled);
    a.cycle().await.unwrap();
    assert!(m.hits("/api/agent/heartbeat").is_empty());
    assert!(a.sync_now().await.is_err());
    a.set_enabled(true).await;
    a.sync_now().await.unwrap();
    assert_eq!(m.hits("/api/agent/heartbeat").len(), 1);
    assert!(m.hits("/api/agent/heartbeat")[0].body.get("heartbeatSeconds").is_none());
    assert_eq!(a.set_heartbeat_seconds(Some(5)).await.heartbeat_seconds, 15, "clamped");
    a.sync_now().await.unwrap();
    assert_eq!(m.hits("/api/agent/heartbeat")[1].body["heartbeatSeconds"], 15);
    assert_eq!(a.status().heartbeat_seconds, 15, "the console's echo does not override the local choice");
}

#[test]
fn templates_and_urls() {
    let t = prepare_template(&json!({
        "name": "x", "id": "evil", "stats": {}, "passphraseCredentialId": "crd_9",
        "destination": { "type": "cloud", "credential_id": "central:crd_2" },
        "scripts": { "env": ["central:crd_3", "central:../bad"] }
    }))
    .unwrap();
    assert_eq!(t.passphrase.as_deref(), Some("$central:crd_9"));
    assert!(!t.fields.contains_key("id") && !t.fields.contains_key("stats") && !t.fields.contains_key("passphraseCredentialId"));
    assert_eq!(t.fields["destination"]["credential_id"], "central:crd_2");
    assert_eq!(t.fields["scripts"]["env"], json!(["$central:crd_3", "central:../bad"]));
    assert_eq!(t.credentials, vec!["central:crd_9", "central:crd_2", "central:crd_3"]);
    assert!(prepare_template(&json!([1])).is_err());
    assert_eq!(managed_job_id("fpl_1").unwrap(), "policy-fpl_1");
    assert!(managed_job_id("../x").is_err());
    assert_eq!(policy_of_job("policy-fpl_1"), Some("fpl_1"));
    assert_eq!(policy_of_job("job-1"), None);

    assert_eq!(normalize_console_url("http://localhost:3001/").unwrap(), "http://localhost:3001");
    assert_eq!(normalize_console_url("https://c.example.com/base").unwrap(), "https://c.example.com/base");
    assert!(normalize_console_url("http://c.example.com").is_err());
    assert!(normalize_console_url("ftp://c.example.com").is_err());
    assert!(normalize_console_url("not a url").is_err());
    assert_eq!(parse_tags("B, a,,a c"), vec!["a", "b", "c"]);
}
