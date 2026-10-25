//! In-app feedback (Help > Send feedback): a message and a category, with an
//! optional scrubbed diagnostics bundle (off unless the person ticks it).
//!
//! Where it goes: to the management console this machine is registered with,
//! over the same endpoint diagnostics use, as a small zip holding
//! `feedback.json` (and, when asked for, the usual `system.json` and scrubbed
//! logs); on a machine with no console, only to a file the person chooses.
//! Nothing is reported as sent unless the console accepted it, and a failed
//! send is an error the person can answer by saving to a file instead.

use crate::core::error::AppError;
use crate::diagnostics::{self, Bundle, SavedBundle};
use crate::fleet_agent::FleetAgent;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::Write;
use std::path::Path;
use ufop_core::agent::DiagnosticsUpload;

/// Longest message accepted (characters).
pub const MAX_MESSAGE_CHARS: usize = 5_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Bug,
    Idea,
    Question,
    Other,
}

impl Category {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bug => "bug",
            Self::Idea => "idea",
            Self::Question => "question",
            Self::Other => "other",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Request {
    pub category: Category,
    pub message: String,
    /// Add the scrubbed diagnostics bundle (logs, versions, OS). Off unless set.
    #[serde(default)]
    pub include_diagnostics: bool,
    /// Where to save it when this machine has no console: a folder or a file.
    #[serde(default)]
    pub save_path: Option<String>,
    /// Save to `save_path` even though a console is configured (the answer to
    /// a send that failed).
    #[serde(default)]
    pub save_only: bool,
}

/// Where feedback goes on this machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Destination {
    /// Sent to the management console this machine is registered with.
    Console { console_url: String, organization: String },
    /// Saved to a file the person picks (no console is configured).
    File,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Outcome {
    /// The console accepted it.
    Sent { id: String, console_url: String, includes_diagnostics: bool },
    /// Written to disk; nothing was sent anywhere.
    Saved { path: String, size: u64, includes_diagnostics: bool },
}

pub fn destination(agent: &FleetAgent) -> Destination {
    let st = agent.status();
    if st.registered {
        Destination::Console { console_url: st.console_url, organization: st.organization }
    } else {
        Destination::File
    }
}

fn check(req: &Request) -> Result<String, AppError> {
    let message = req.message.trim().to_string();
    if message.is_empty() {
        return Err(AppError::validation("Write a message first"));
    }
    if message.chars().count() > MAX_MESSAGE_CHARS {
        return Err(AppError::validation(format!("The message is longer than {MAX_MESSAGE_CHARS} characters; shorten it")));
    }
    Ok(message)
}

/// A zip with just `files`.
fn zip_of(files: &[(String, String)]) -> Result<Vec<u8>, AppError> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, text) in files {
        zip.start_file(name.as_str(), opts).map_err(|e| AppError::internal(format!("Cannot write the feedback: {e}")))?;
        zip.write_all(text.as_bytes())?;
    }
    Ok(zip.finish().map_err(|e| AppError::internal(format!("Cannot write the feedback: {e}")))?.into_inner())
}

/// The feedback as the zip that is sent or saved. With `include_diagnostics`
/// the zip also holds `system.json` and the scrubbed logs; without, only the
/// message, its category, the app version and the OS family.
pub fn prepare(req: &Request, log_dir: &Path) -> Result<(DiagnosticsUpload, Bundle), AppError> {
    let message = check(req)?;
    let info = ufop_core::agent::ClientInfo::detect("desktop", env!("CARGO_PKG_VERSION"));
    let feedback = json!({
        "kind": "feedback",
        "category": req.category.as_str(),
        "message": message,
        "app": { "name": "UFOP", "version": env!("CARGO_PKG_VERSION") },
        "os": info.os,
        "osVersion": info.os_version,
        "includesDiagnostics": req.include_diagnostics,
        "createdAt": chrono::Utc::now().to_rfc3339(),
    });
    let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
    let one_line: String = message.split_whitespace().collect::<Vec<_>>().join(" ");
    let preview: String = one_line.chars().take(200).collect();
    let mut summary = json!({ "kind": "feedback", "category": req.category.as_str(), "preview": preview, "appVersion": env!("CARGO_PKG_VERSION"), "os": info.os, "includesDiagnostics": req.include_diagnostics });
    let feedback_file = ("feedback.json".to_string(), serde_json::to_string_pretty(&feedback)?);
    let bundle = if req.include_diagnostics {
        let system = diagnostics::system_info("desktop");
        let mut b = diagnostics::build_with(log_dir, &system, diagnostics::MAX_LOG_BYTES, &[feedback_file])?;
        b.file_name = format!("ufop-feedback-{stamp}.zip");
        if let (Some(s), Some(extra)) = (summary.as_object_mut(), diagnostics::summary(&b, &system).as_object()) {
            for (k, v) in extra {
                s.entry(k.clone()).or_insert_with(|| v.clone());
            }
        }
        b
    } else {
        Bundle { file_name: format!("ufop-feedback-{stamp}.zip"), files: vec![feedback_file.0.clone()], bytes: zip_of(&[feedback_file])?, ..Default::default() }
    };
    let upload = DiagnosticsUpload { file_name: bundle.file_name.clone(), summary: summary as Value, bytes: bundle.bytes.clone() };
    Ok((upload, bundle))
}

/// Send the feedback to the console when this machine has one; otherwise save
/// it to `save_path`. A console that cannot be reached is an error (the person
/// can then save it instead), never a silent save or a pretend send.
pub async fn submit(agent: &FleetAgent, req: Request, log_dir: &Path) -> Result<Outcome, AppError> {
    let dir = log_dir.to_path_buf();
    let request = req.clone();
    let (upload, bundle) = tokio::task::spawn_blocking(move || prepare(&request, &dir))
        .await
        .map_err(|e| AppError::internal(format!("Feedback task failed: {e}")))??;
    let destination = if req.save_only { Destination::File } else { destination(agent) };
    match destination {
        Destination::Console { .. } => {
            let (id, console_url) = agent.send_bundle(&upload).await.map_err(|e| {
                AppError::connection(format!("The feedback was not sent: {}", e.user_message()), "Check the connection to the console and try again, or save it to a file instead.")
            })?;
            Ok(Outcome::Sent { id, console_url, includes_diagnostics: req.include_diagnostics })
        }
        Destination::File => {
            let target = req.save_path.as_deref().map(str::trim).filter(|p| !p.is_empty()).ok_or_else(|| {
                AppError::validation("This computer is not connected to a management console. Choose where to save the feedback.")
            })?;
            let SavedBundle { path, size, .. } = diagnostics::save(&bundle, Path::new(target))?;
            Ok(Outcome::Saved { path, size, includes_diagnostics: req.include_diagnostics })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn req(msg: &str, diagnostics: bool) -> Request {
        Request { category: Category::Bug, message: msg.into(), include_diagnostics: diagnostics, save_path: None, save_only: false }
    }

    fn files_of(bytes: &[u8]) -> std::collections::BTreeMap<String, String> {
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes.to_vec())).unwrap();
        (0..z.len())
            .map(|i| {
                let mut f = z.by_index(i).unwrap();
                let mut s = String::new();
                f.read_to_string(&mut s).unwrap();
                (f.name().to_string(), s)
            })
            .collect()
    }

    fn logs() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("ufop.2026-10-24.log"), "2026-10-24T10:00:00Z  INFO app: password=hunter2 opened /home/bob/secret.txt for bob@example.com\n").unwrap();
        d
    }

    #[test]
    fn without_diagnostics_only_the_message_and_the_basics_leave() {
        let d = logs();
        let (upload, bundle) = prepare(&req("  The sync stalls at 99%  ", false), d.path()).unwrap();
        let files = files_of(&upload.bytes);
        assert_eq!(files.keys().collect::<Vec<_>>(), ["feedback.json"], "no logs, no system details");
        let fb: Value = serde_json::from_str(&files["feedback.json"]).unwrap();
        assert_eq!((fb["category"].as_str(), fb["message"].as_str(), fb["includesDiagnostics"].as_bool()), (Some("bug"), Some("The sync stalls at 99%"), Some(false)));
        assert!(fb["app"]["version"].is_string() && fb["os"].is_string());
        assert_eq!(upload.summary["kind"], "feedback");
        assert_eq!(upload.summary["includesDiagnostics"], false);
        assert_eq!(bundle.bytes, upload.bytes);
        assert!(upload.file_name.starts_with("ufop-feedback-") && upload.file_name.ends_with(".zip"));
    }

    #[test]
    fn with_diagnostics_the_scrubbed_logs_and_system_details_come_too() {
        let d = logs();
        let (upload, _) = prepare(&req("It crashed", true), d.path()).unwrap();
        let files = files_of(&upload.bytes);
        assert!(files.contains_key("feedback.json") && files.contains_key("system.json"));
        let log = files.iter().find(|(k, _)| k.starts_with("logs/")).map(|(_, v)| v.clone()).expect("a log file");
        for leak in ["hunter2", "bob@example.com", "/home/bob"] {
            assert!(!log.contains(leak), "{leak} leaked: {log}");
        }
        assert!(log.contains("secret.txt"));
        assert_eq!(upload.summary["includesDiagnostics"], true);
        assert!(upload.summary["logBytes"].is_number(), "the console sees the bundle summary as well");
    }

    #[test]
    fn an_empty_or_huge_message_is_refused() {
        let d = logs();
        assert!(prepare(&req("   \n ", false), d.path()).unwrap_err().user_message().contains("Write a message"));
        assert!(prepare(&req(&"x".repeat(MAX_MESSAGE_CHARS + 1), false), d.path()).unwrap_err().user_message().contains("shorten"));
        assert!(prepare(&req(&"x".repeat(MAX_MESSAGE_CHARS), false), d.path()).is_ok());
    }

    #[test]
    fn categories_use_their_stored_names() {
        for (c, s) in [(Category::Bug, "bug"), (Category::Idea, "idea"), (Category::Question, "question"), (Category::Other, "other")] {
            assert_eq!(c.as_str(), s);
            assert_eq!(serde_json::to_value(c).unwrap(), json!(s));
        }
        assert!(serde_json::from_value::<Request>(json!({ "category": "rant", "message": "x" })).is_err());
        let r: Request = serde_json::from_value(json!({ "category": "idea", "message": "x" })).unwrap();
        assert!(!r.include_diagnostics, "diagnostics are off unless asked for");
    }

    // ── The two destinations, with a real agent and a stub console ──

    use std::sync::{Arc, Mutex};
    use ufop_core::agent::mock::{MockConsole, MockRequest};

    async fn agent_of(dir: &Path) -> FleetAgent {
        let repo = crate::storage::Repository::open_in_memory().await.unwrap();
        let creds = Arc::new(crate::security::CredentialStore::new());
        let mgr = crate::backup_engine::BackupManager::with_kdf(repo.pool().clone(), creds.clone(), None, None, dir.join("dbs"), ufop_core::backup::crypto::KdfParams::insecure_fast());
        let settings = crate::security::settings_store::SettingsStore::plain(repo.pool().clone());
        mgr.set_settings_store(settings.clone());
        crate::fleet_agent::init(mgr, settings, creds, "desktop").await.unwrap()
    }

    /// A console that accepts registration and diagnostics uploads; `accept` false answers 503.
    fn console(accept: bool) -> (MockConsole, Arc<Mutex<Vec<Value>>>) {
        let got: Arc<Mutex<Vec<Value>>> = Arc::default();
        let sink = got.clone();
        let m = MockConsole::start(move |r: &MockRequest| match r.path.as_str() {
            "/api/agent/register" => (201, json!({ "machineId": "mch_1", "agentToken": "ufop_agent_t1", "organization": "Acme", "heartbeatSeconds": 60 })),
            "/api/agent/diagnostics" if accept && r.bearer() == Some("ufop_agent_t1") => {
                sink.lock().unwrap().push(r.body.clone());
                (201, json!({ "id": "dgn_7" }))
            }
            "/api/agent/diagnostics" => (503, json!({ "error": "maintenance" })),
            _ => (404, json!({ "error": "not found" })),
        })
        .unwrap();
        (m, got)
    }

    #[tokio::test]
    async fn on_a_machine_without_a_console_feedback_is_saved_to_a_file_and_says_so() {
        let t = tempfile::tempdir().unwrap();
        let agent = agent_of(t.path()).await;
        assert_eq!(destination(&agent), Destination::File);
        let logs = logs();

        // No place to save it: asked for one, nothing written.
        let err = submit(&agent, req("Love it", false), logs.path()).await.unwrap_err();
        assert!(err.user_message().contains("Choose where to save"), "{}", err.user_message());

        let out_dir = t.path().join("out");
        std::fs::create_dir_all(&out_dir).unwrap();
        let outcome = submit(&agent, Request { save_path: Some(out_dir.to_string_lossy().into()), ..req("Love it", false) }, logs.path()).await.unwrap();
        let Outcome::Saved { path, size, includes_diagnostics } = outcome else { panic!("expected a saved file, not a send") };
        assert!(!includes_diagnostics && size > 0);
        let files = files_of(&std::fs::read(&path).unwrap());
        assert_eq!(files.keys().collect::<Vec<_>>(), ["feedback.json"]);
        assert!(path.starts_with(&*out_dir.to_string_lossy()));
    }

    #[tokio::test]
    async fn on_a_registered_machine_feedback_goes_to_the_console_and_never_to_disk() {
        let t = tempfile::tempdir().unwrap();
        let agent = agent_of(t.path()).await;
        let (m, got) = console(true);
        agent.register(&m.url, "ufop_reg_x", vec![]).await.unwrap();
        assert!(matches!(destination(&agent), Destination::Console { ref organization, .. } if organization == "Acme"));
        let logs = logs();
        let out_dir = t.path().join("out");
        std::fs::create_dir_all(&out_dir).unwrap();

        let outcome = submit(&agent, Request { category: Category::Idea, save_path: Some(out_dir.to_string_lossy().into()), ..req("Add dark mode", true) }, logs.path()).await.unwrap();

        let Outcome::Sent { id, console_url, includes_diagnostics } = outcome else { panic!("expected a send") };
        assert_eq!((id.as_str(), includes_diagnostics), ("dgn_7", true));
        assert_eq!(console_url, m.url);
        assert_eq!(std::fs::read_dir(&out_dir).unwrap().count(), 0, "nothing is written when it is sent");
        let upload = got.lock().unwrap()[0].clone();
        assert_eq!(upload["summary"]["kind"], "feedback");
        assert_eq!(upload["summary"]["category"], "idea");
        assert!(upload["fileName"].as_str().unwrap().starts_with("ufop-feedback-"));
        use base64::Engine as _;
        let bytes = base64::engine::general_purpose::STANDARD.decode(upload["bundleBase64"].as_str().unwrap()).unwrap();
        let files = files_of(&bytes);
        assert!(files["feedback.json"].contains("Add dark mode") && files.contains_key("system.json"));
    }

    #[tokio::test]
    async fn after_a_failed_send_the_person_can_save_it_instead() {
        let t = tempfile::tempdir().unwrap();
        let agent = agent_of(t.path()).await;
        let (m, got) = console(false);
        agent.register(&m.url, "ufop_reg_x", vec![]).await.unwrap();
        let logs = logs();
        let out_dir = t.path().join("out");
        std::fs::create_dir_all(&out_dir).unwrap();
        let ask = Request { save_path: Some(out_dir.to_string_lossy().into()), save_only: true, ..req("Hello", false) };

        let Outcome::Saved { path, .. } = submit(&agent, ask, logs.path()).await.unwrap() else { panic!("expected a saved file") };

        assert!(std::path::Path::new(&path).exists());
        assert!(got.lock().unwrap().is_empty(), "saving does not contact the console");
    }

    #[tokio::test]
    async fn a_console_that_refuses_is_an_error_not_a_pretend_send_or_a_silent_save() {
        let t = tempfile::tempdir().unwrap();
        let agent = agent_of(t.path()).await;
        let (m, got) = console(false);
        agent.register(&m.url, "ufop_reg_x", vec![]).await.unwrap();
        let logs = logs();
        let out_dir = t.path().join("out");
        std::fs::create_dir_all(&out_dir).unwrap();

        let err = submit(&agent, Request { save_path: Some(out_dir.to_string_lossy().into()), ..req("Hello", false) }, logs.path()).await.unwrap_err();

        assert!(err.user_message().contains("was not sent"), "{}", err.user_message());
        assert!(got.lock().unwrap().is_empty());
        assert_eq!(std::fs::read_dir(&out_dir).unwrap().count(), 0, "it is not saved behind the person's back");
    }
}
