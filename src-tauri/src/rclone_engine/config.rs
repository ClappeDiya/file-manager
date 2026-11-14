//! The in-app rclone remote manager.
//!
//! - Provider schemas come from `rclone config providers` (JSON), turned
//!   into [`ProviderSchema`] so the UI can generate a form for every backend
//!   the installed rclone has.
//! - Remotes are created and updated with
//!   `rclone config create|update <name> [<type>] key=value… --non-interactive`.
//!   When rclone needs an answer it returns a question (a [`ConfigStep`]);
//!   the answer goes back with `--continue --state … --result …` — rclone's
//!   documented non-interactive state machine. No interactive TTY is used.
//! - OAuth backends ask "Use auto config?" (`config_is_local`).
//!   [`oauth_sign_in`] answers it the headless way and runs the
//!   `rclone authorize …` command rclone names, with
//!   `--auth-no-open-browser`, so the app opens the sign-in address itself
//!   (through the system opener) and then hands the token back to rclone.
//! - Passwords: rclone obscures password fields it is given; the UI asks for
//!   `--obscure` (plain text typed) or `--no-obscure` (already obscured).
//!   Values are passed on rclone's command line, as rclone requires.

use super::resolver::Rclone;
use crate::core::error::AppError;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt};
use tokio_util::sync::CancellationToken;

const QUICK: Duration = Duration::from_secs(60);
/// How long a browser sign-in may take.
pub const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(10 * 60);

// ── Provider schemas ──

/// One choice for an option.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OptionExample {
    pub value: String,
    pub help: String,
    /// Provider filter (as for [`OptionSchema::provider`]).
    pub provider: String,
}

/// One configuration option of a backend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OptionSchema {
    pub name: String,
    pub help: String,
    /// Which `provider` values the option applies to: empty = all,
    /// `AWS,Ceph` = those, `!AWS,Ceph` = all but those.
    pub provider: String,
    /// The default, as text.
    pub default: String,
    pub examples: Vec<OptionExample>,
    pub required: bool,
    /// rclone stores it obscured.
    pub is_password: bool,
    /// A secret that is not obscured (keys, tokens): masked in the UI and
    /// never sent back by [`remote_details`].
    pub sensitive: bool,
    pub advanced: bool,
    /// Only the examples are allowed.
    pub exclusive: bool,
    /// Not offered in rclone's own configurator.
    pub hidden: bool,
    #[serde(rename = "type")]
    pub option_type: String,
}

/// A backend (`drive`, `s3`, `crypt`…).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProviderSchema {
    /// The `type` stored in rclone.conf (`google photos`).
    pub name: String,
    pub description: String,
    /// Short name (`gphotos`).
    pub prefix: String,
    pub options: Vec<OptionSchema>,
    pub hidden: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RawExample {
    #[serde(default)]
    value: String,
    #[serde(default)]
    help: String,
    #[serde(default)]
    provider: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RawOption {
    name: String,
    #[serde(default)]
    help: String,
    #[serde(default)]
    provider: String,
    #[serde(default)]
    default: serde_json::Value,
    #[serde(default)]
    default_str: Option<String>,
    #[serde(default)]
    examples: Option<Vec<RawExample>>,
    #[serde(default)]
    hide: u32,
    #[serde(default)]
    required: bool,
    #[serde(default)]
    is_password: bool,
    #[serde(default)]
    sensitive: bool,
    #[serde(default)]
    advanced: bool,
    #[serde(default)]
    exclusive: bool,
    #[serde(default, rename = "Type")]
    option_type: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RawProvider {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    prefix: String,
    #[serde(default)]
    options: Option<Vec<RawOption>>,
    #[serde(default)]
    hide: bool,
}

/// Option names that hold secrets rclone does not obscure (rclone older
/// than 1.63 has no `Sensitive` flag).
const SENSITIVE_NAMES: &[&str] = &[
    "token",
    "access_token",
    "refresh_token",
    "auth_token",
    "bearer_token",
    "permanent_token",
    "plex_token",
    "session_token",
    "client_secret",
    "key",
    "api_key",
    "secret_access_key",
    "private_access_key",
    "application_credential_secret",
    "sse_customer_key",
    "sse_customer_key_base64",
    "sas_url",
    "service_account_credentials",
    "key_pem",
    "link_password",
    "authorization",
    // Obscured passwords: `rclone reveal` turns them back into plain text.
    "pass",
    "user_pass",
    "key_file_pass",
    "secret",
    "library_key",
];

pub fn is_sensitive_name(name: &str) -> bool {
    SENSITIVE_NAMES.contains(&name) || name.ends_with("_secret") || name.ends_with("_token") || name.ends_with("_pass") || name.contains("password")
}

const HIDE_CONFIGURATOR: u32 = 4;

impl From<RawOption> for OptionSchema {
    fn from(o: RawOption) -> Self {
        let default = o.default_str.unwrap_or_else(|| match &o.default {
            serde_json::Value::Null => String::new(),
            serde_json::Value::String(s) => s.clone(),
            other => other.to_string(),
        });
        let sensitive = o.sensitive || o.is_password || is_sensitive_name(&o.name);
        Self {
            examples: o.examples.unwrap_or_default().into_iter().map(|e| OptionExample { value: e.value, help: e.help, provider: e.provider }).collect(),
            hidden: o.hide & HIDE_CONFIGURATOR != 0,
            name: o.name,
            help: o.help,
            provider: o.provider,
            default,
            required: o.required,
            is_password: o.is_password,
            sensitive,
            advanced: o.advanced,
            exclusive: o.exclusive,
            option_type: o.option_type,
        }
    }
}

/// `rclone config providers` → schemas (sorted by name).
pub fn parse_providers(json: &str) -> Result<Vec<ProviderSchema>, AppError> {
    let raw: Vec<RawProvider> = serde_json::from_str(json).map_err(|e| AppError::connection(format!("Unexpected `rclone config providers` output: {e}"), "Update rclone."))?;
    let mut v: Vec<ProviderSchema> = raw
        .into_iter()
        .map(|p| ProviderSchema {
            prefix: if p.prefix.is_empty() { p.name.clone() } else { p.prefix },
            name: p.name,
            description: p.description,
            options: p.options.unwrap_or_default().into_iter().map(OptionSchema::from).collect(),
            hidden: p.hide,
        })
        .collect();
    v.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(v)
}

type ProviderCache = Mutex<Option<(String, Arc<Vec<ProviderSchema>>)>>;

fn provider_cache() -> &'static ProviderCache {
    static C: OnceLock<ProviderCache> = OnceLock::new();
    C.get_or_init(|| Mutex::new(None))
}

/// The binary's identity for caching (path, size, modification time).
fn binary_key(program: &str) -> String {
    let meta = super::resolver::which(program).or_else(|| Some(program.into())).and_then(|p| std::fs::metadata(p).ok());
    let stamp = meta.map(|m| format!("{}:{:?}", m.len(), m.modified().ok())).unwrap_or_default();
    format!("{program}|{stamp}")
}

/// Every backend the rclone binary supports (cached per binary).
pub async fn providers(rc: &Rclone) -> Result<Arc<Vec<ProviderSchema>>, AppError> {
    let key = binary_key(&rc.program);
    if let Some((k, v)) = provider_cache().lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        if *k == key {
            return Ok(v.clone());
        }
    }
    let out = rc.run(&["config".into(), "providers".into()], QUICK).await?;
    let v = Arc::new(parse_providers(&out)?);
    *provider_cache().lock().unwrap_or_else(|e| e.into_inner()) = Some((key, v.clone()));
    Ok(v)
}

/// The schema for a remote type (by name or prefix).
pub fn find_provider<'a>(list: &'a [ProviderSchema], remote_type: &str) -> Option<&'a ProviderSchema> {
    list.iter().find(|p| p.name == remote_type || p.prefix == remote_type)
}

// ── Validation ──

/// rclone remote names: letters, digits, `_ . + @ -` and inner spaces.
pub fn validate_name(name: &str) -> Result<String, AppError> {
    let n = name.trim().trim_end_matches(':');
    let ok = !n.is_empty()
        && n.chars().count() <= 128
        && !n.starts_with(['-', ' '])
        && n.chars().all(|c| c.is_alphanumeric() || "_.+@- ".contains(c));
    if !ok {
        return Err(AppError::validation(format!("\"{name}\" is not a valid rclone remote name: use letters, digits, _ . + @ - (not starting with - or a space)")));
    }
    Ok(n.to_string())
}

fn validate_key(key: &str) -> Result<(), AppError> {
    if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(AppError::validation(format!("\"{key}\" is not a valid rclone option name")));
    }
    Ok(())
}

/// `key=value` arguments (validated: option names only, single-line values).
pub fn param_args(params: &BTreeMap<String, String>) -> Result<Vec<String>, AppError> {
    params
        .iter()
        .map(|(k, v)| {
            validate_key(k)?;
            if v.contains(['\n', '\r']) {
                return Err(AppError::validation(format!("The value of {k} must be on one line")));
            }
            Ok(format!("{k}={v}"))
        })
        .collect()
}

/// How rclone should treat password values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObscureMode {
    /// rclone decides (it obscures values that do not look obscured).
    #[default]
    Auto,
    /// The values are plain text: always obscure (`--obscure`).
    Obscure,
    /// The values are already obscured (`--no-obscure`).
    AlreadyObscured,
}

impl ObscureMode {
    fn flag(self) -> Option<&'static str> {
        match self {
            Self::Auto => None,
            Self::Obscure => Some("--obscure"),
            Self::AlreadyObscured => Some("--no-obscure"),
        }
    }
}

// ── The state machine ──

/// A step of `rclone config create|update --non-interactive`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConfigStep {
    /// Pass back with the answer; empty when finished.
    pub state: String,
    /// The question to ask, if any.
    pub option: Option<OptionSchema>,
    /// Shown with the question (a rejected answer).
    pub error: String,
    pub result: String,
    pub done: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RawStep {
    #[serde(default)]
    state: String,
    #[serde(default)]
    option: Option<RawOption>,
    #[serde(default)]
    error: String,
    #[serde(default)]
    result: String,
}

/// rclone's JSON answer (it may be preceded by log lines).
pub fn parse_step(out: &str) -> Result<ConfigStep, AppError> {
    let start = out.find('{').ok_or_else(|| AppError::connection("rclone did not return a configuration step", "Update rclone (1.56 or newer is needed)."))?;
    let raw: RawStep = serde_json::from_str(out[start..].trim()).map_err(|e| AppError::connection(format!("Unexpected rclone configuration output: {e}"), "Update rclone."))?;
    Ok(ConfigStep { done: raw.state.is_empty(), state: raw.state, option: raw.option.map(OptionSchema::from), error: raw.error, result: raw.result })
}

/// Create remote `name` of `remote_type` with `params`.
pub async fn create(rc: &Rclone, name: &str, remote_type: &str, params: &BTreeMap<String, String>, obscure: ObscureMode) -> Result<ConfigStep, AppError> {
    let name = validate_name(name)?;
    let list = providers(rc).await?;
    let p = find_provider(&list, remote_type).ok_or_else(|| AppError::validation(format!("This rclone has no \"{remote_type}\" backend")))?;
    if list_remotes(rc).await?.iter().any(|r| r.name.trim_end_matches(':') == name) {
        return Err(AppError::validation(format!("An rclone remote named \"{name}\" already exists")));
    }
    let mut args: Vec<String> = vec!["config".into(), "create".into(), name, p.name.clone()];
    args.extend(param_args(params)?);
    args.push("--non-interactive".into());
    args.extend(obscure.flag().map(String::from));
    parse_step(&rc.run(&args, QUICK).await?)
}

/// Change options of remote `name`.
pub async fn update(rc: &Rclone, name: &str, params: &BTreeMap<String, String>, obscure: ObscureMode) -> Result<ConfigStep, AppError> {
    let name = existing(rc, name).await?;
    let mut args: Vec<String> = vec!["config".into(), "update".into(), name];
    args.extend(param_args(params)?);
    args.push("--non-interactive".into());
    args.extend(obscure.flag().map(String::from));
    parse_step(&rc.run(&args, QUICK).await?)
}

/// `obscure` must be the create/update call's mode: the params (plain
/// secrets included) are written again, and without the flag rclone
/// guesses, storing a base64-looking password raw.
fn continue_args(name: &str, state: &str, result: &str, params: &BTreeMap<String, String>, obscure: ObscureMode) -> Result<Vec<String>, AppError> {
    if result.contains(['\n', '\r']) && !result.trim_start().starts_with('{') {
        return Err(AppError::validation("The answer must be on one line"));
    }
    let mut args: Vec<String> = vec!["config".into(), "update".into(), name.to_string()];
    args.extend(param_args(params)?);
    // `--flag=value` so an answer starting with "-" is never read as a flag.
    args.extend(["--continue".into(), "--non-interactive".into(), format!("--state={state}"), format!("--result={result}")]);
    args.extend(obscure.flag().map(String::from));
    Ok(args)
}

/// Answer the question of a step (`result`) and get the next one; `params`
/// and `obscure` are those of the create/update call that started it.
pub async fn continue_with(rc: &Rclone, name: &str, state: &str, result: &str, params: &BTreeMap<String, String>, obscure: ObscureMode) -> Result<ConfigStep, AppError> {
    let name = existing(rc, name).await?;
    parse_step(&rc.run(&continue_args(&name, state, result, params, obscure)?, QUICK).await?)
}

/// The remote's name if it is configured.
async fn existing(rc: &Rclone, name: &str) -> Result<String, AppError> {
    let name = validate_name(name)?;
    if !list_remotes(rc).await?.iter().any(|r| r.name.trim_end_matches(':') == name) {
        return Err(AppError::not_found(format!("No rclone remote named \"{name}\"")));
    }
    Ok(name)
}

/// Remove remote `name` from rclone.conf.
pub async fn delete(rc: &Rclone, name: &str) -> Result<(), AppError> {
    let name = existing(rc, name).await?;
    rc.run(&["config".into(), "delete".into(), name], QUICK).await.map(|_| ())
}

/// Configured remotes (`rclone listremotes --long`).
pub async fn list_remotes(rc: &Rclone) -> Result<Vec<crate::connectors::rclone::RcloneRemote>, AppError> {
    let out = rc.run(&["listremotes".into(), "--long".into()], QUICK).await?;
    Ok(crate::connectors::rclone::parse_listremotes(&out))
}

/// Every remote's options (`rclone config dump`; stays in the backend).
pub async fn dump(rc: &Rclone) -> Result<BTreeMap<String, BTreeMap<String, String>>, AppError> {
    let out = rc.run(&["config".into(), "dump".into()], QUICK).await?;
    let v: BTreeMap<String, BTreeMap<String, serde_json::Value>> = serde_json::from_str(out.trim()).map_err(|e| AppError::connection(format!("Unexpected `rclone config dump` output: {e}"), "Update rclone."))?;
    Ok(v.into_iter()
        .map(|(k, m)| (k, m.into_iter().map(|(ok, ov)| (ok, ov.as_str().map(str::to_string).unwrap_or_else(|| ov.to_string()))).collect()))
        .collect())
}

/// A remote's settings for the edit form; secrets are not included.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RemoteDetails {
    pub name: String,
    #[serde(rename = "type")]
    pub remote_type: String,
    /// Non-secret options as stored.
    pub values: BTreeMap<String, String>,
    /// Secret options that are set (their values are withheld).
    pub secrets_set: Vec<String>,
}

/// Split `section` into shown values and withheld secrets.
pub fn redact(name: &str, section: &BTreeMap<String, String>, schema: Option<&ProviderSchema>) -> RemoteDetails {
    let secret = |k: &str| schema.is_some_and(|s| s.options.iter().any(|o| o.name == k && (o.is_password || o.sensitive))) || is_sensitive_name(k);
    let mut d = RemoteDetails { name: name.to_string(), remote_type: section.get("type").cloned().unwrap_or_default(), values: BTreeMap::new(), secrets_set: Vec::new() };
    for (k, v) in section {
        if k == "type" {
            continue;
        }
        if secret(k) {
            if !v.is_empty() {
                d.secrets_set.push(k.clone());
            }
        } else {
            d.values.insert(k.clone(), v.clone());
        }
    }
    d
}

/// Settings of remote `name` (secrets withheld).
pub async fn remote_details(rc: &Rclone, name: &str) -> Result<RemoteDetails, AppError> {
    let name = validate_name(name)?;
    let all = dump(rc).await?;
    let section = all.get(&name).ok_or_else(|| AppError::not_found(format!("No rclone remote named \"{name}\"")))?;
    let list = providers(rc).await.ok();
    let schema = list.as_ref().and_then(|l| find_provider(l, section.get("type").map(String::as_str).unwrap_or("")).cloned());
    Ok(redact(&name, section, schema.as_ref()))
}

/// Outcome of listing a remote's root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RemoteTest {
    pub ok: bool,
    /// Entries at the root.
    pub entries: usize,
    pub error: Option<String>,
    pub millis: u64,
}

/// List the root of remote `name` (one level).
pub async fn test_remote(rc: &Rclone, name: &str) -> Result<RemoteTest, AppError> {
    let name = validate_name(name)?;
    let t = std::time::Instant::now();
    let r = rc.run(&["lsjson".into(), "--max-depth".into(), "1".into(), "--no-mimetype".into(), "--no-modtime".into(), format!("{name}:")], QUICK).await;
    let millis = t.elapsed().as_millis() as u64;
    Ok(match r {
        Ok(out) => RemoteTest { ok: true, entries: serde_json::from_str::<Vec<serde_json::Value>>(out.trim()).map(|v| v.len()).unwrap_or(0), error: None, millis },
        Err(e) => RemoteTest { ok: false, entries: 0, error: Some(e.user_message()), millis },
    })
}

// ── OAuth sign-in ──

/// Sign-ins in progress, by remote name (for cancelling): the flow's id
/// and its cancel token.
fn flows() -> &'static Mutex<HashMap<String, (u64, CancellationToken)>> {
    static F: OnceLock<Mutex<HashMap<String, (u64, CancellationToken)>>> = OnceLock::new();
    F.get_or_init(Default::default)
}

/// Track a new sign-in for `name`, stopping one already running for it (a
/// single remote has one sign-in; the old one also holds rclone's callback
/// port). Untracked when the returned guard drops, unless superseded.
fn track_sign_in(name: &str, token: CancellationToken) -> impl Drop {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    if let Some((_, old)) = flows().lock().unwrap_or_else(|e| e.into_inner()).insert(name.to_string(), (id, token)) {
        old.cancel();
    }
    struct Tracked(String, u64);
    impl Drop for Tracked {
        fn drop(&mut self) {
            let mut f = flows().lock().unwrap_or_else(|e| e.into_inner());
            if f.get(&self.0).is_some_and(|(id, _)| *id == self.1) {
                f.remove(&self.0);
            }
        }
    }
    Tracked(name.to_string(), id)
}

/// Stop the sign-in of `name`; true if one was running.
pub fn cancel_sign_in(name: &str) -> bool {
    let name = name.trim().trim_end_matches(':');
    flows().lock().unwrap_or_else(|e| e.into_inner()).remove(name).map(|(_, t)| t.cancel()).is_some()
}

/// Whether a step asks rclone's "Use auto config?" OAuth question.
pub fn is_oauth_question(step: &ConfigStep) -> bool {
    step.option.as_ref().is_some_and(|o| o.name == "config_is_local") || step.state.starts_with("*oauth-islocal")
}

/// The `rclone authorize …` arguments from the `config_token` question
/// (`\trclone authorize "drive" "eyJ…"` in its help).
pub fn authorize_args(help: &str) -> Option<Vec<String>> {
    let line = help.lines().map(str::trim).find(|l| l.starts_with("rclone authorize"))?;
    let words = shlex::split(line)?;
    let args: Vec<String> = words.into_iter().skip(2).collect();
    (!args.is_empty() && !args.iter().any(|a| a.starts_with('-'))).then_some(args)
}

/// The sign-in address rclone printed (`Please go to the following link: …`).
pub fn auth_url(line: &str) -> Option<String> {
    let at = line.find("http://").or_else(|| line.find("https://"))?;
    let url = line[at..].split_whitespace().next()?.trim_end_matches(['.', ',']);
    (line.contains("following link") || url.contains("/auth")).then(|| url.to_string())
}

/// The token from `rclone authorize` output.
pub fn authorize_token(stdout: &str) -> Option<String> {
    if let Some((_, rest)) = stdout.split_once("--->") {
        let token = rest.split("<---").next()?.trim();
        return (!token.is_empty()).then(|| token.to_string());
    }
    stdout.lines().map(str::trim).rfind(|l| l.starts_with('{') && serde_json::from_str::<serde_json::Value>(l).is_ok()).map(str::to_string)
}

/// Run `rclone authorize <args> --auth-no-open-browser`: `on_url` gets the
/// sign-in address as soon as rclone prints it; the token comes back once
/// the browser returns. Stopped by `cancel` or after `timeout`.
pub async fn authorize(rc: &Rclone, args: &[String], cancel: CancellationToken, timeout: Duration, on_url: &(dyn Fn(String) + Send + Sync)) -> Result<String, AppError> {
    let mut cmd = rc.command();
    cmd.arg("authorize").args(args).arg("--auth-no-open-browser").stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
    let mut child = super::resolver::spawn(&mut cmd).await.map_err(|e| AppError::connection(format!("Could not start rclone: {e}"), "Check the rclone binary in Settings → rclone."))?;
    let (mut out, err) = (child.stdout.take(), child.stderr.take());
    let read_err = async {
        let mut tail = String::new();
        if let Some(e) = err {
            let mut lines = tokio::io::BufReader::new(e).lines();
            let mut sent = false;
            while let Ok(Some(l)) = lines.next_line().await {
                if !sent {
                    if let Some(u) = auth_url(&l) {
                        sent = true;
                        on_url(u);
                    }
                }
                tail = l;
            }
        }
        tail
    };
    let read_out = async {
        let mut s = String::new();
        if let Some(o) = out.as_mut() {
            let _ = o.read_to_string(&mut s).await;
        }
        s
    };
    let work = async {
        let (tail, stdout) = tokio::join!(read_err, read_out);
        let status = child.wait().await;
        (tail, stdout, status)
    };
    let (tail, stdout, status) = tokio::select! {
        r = work => r,
        _ = cancel.cancelled() => return Err(AppError::state("Sign-in cancelled", "Start the sign-in again when you are ready.")),
        _ = tokio::time::sleep(timeout) => return Err(AppError::connection("The sign-in was not completed in time", "Start the sign-in again and finish it in the browser.")),
    };
    let ok = status.map(|s| s.success()).unwrap_or(false);
    match authorize_token(&stdout) {
        Some(t) if ok => Ok(t),
        _ => Err(AppError::auth(format!("rclone sign-in failed: {}", super::resolver::last_error_line(&tail)), "Try the sign-in again; check the account and that the browser reached the confirmation page.")),
    }
}

/// Finish an OAuth question (`step` asks `config_is_local`) by signing in
/// through the browser: answer "headless", run the `rclone authorize`
/// command rclone asks for, then hand rclone the token. Returns the next
/// step (usually done, or the backend's next question).
pub async fn oauth_sign_in(rc: &Rclone, name: &str, state: &str, params: &BTreeMap<String, String>, obscure: ObscureMode, on_url: &(dyn Fn(String) + Send + Sync)) -> Result<ConfigStep, AppError> {
    let name = existing(rc, name).await?;
    let token = CancellationToken::new();
    let _tracked = track_sign_in(&name, token.clone());
    let headless = parse_step(&rc.run(&continue_args(&name, state, "false", params, obscure)?, QUICK).await?)?;
    let help = headless.option.as_ref().filter(|o| o.name == "config_token").map(|o| o.help.clone()).ok_or_else(|| {
        AppError::connection("rclone did not ask for a sign-in token", "Answer the questions manually (choose \"No\" for auto config and paste a token from `rclone authorize`).")
    })?;
    let args = authorize_args(&help).ok_or_else(|| AppError::connection("rclone did not name the authorize command", "Update rclone."))?;
    let tok = authorize(rc, &args, token, SIGN_IN_TIMEOUT, on_url).await?;
    parse_step(&rc.run(&continue_args(&name, &headless.state, &tok, params, obscure)?, QUICK).await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROVIDERS_FIXTURE: &str = include_str!("fixtures/providers.json");

    fn rclone_available() -> bool {
        std::process::Command::new("rclone").arg("version").output().is_ok_and(|o| o.status.success())
    }

    /// A real rclone with an empty temp config.
    fn temp_rclone() -> Option<(tempfile::TempDir, Rclone)> {
        if !rclone_available() {
            return None;
        }
        let d = tempfile::tempdir().unwrap();
        let conf = d.path().join("rclone.conf");
        Some((d, Rclone::new("rclone", Some(conf.to_string_lossy().into_owned()))))
    }

    /// Remotes in the temp config (other tests define remotes through
    /// `RCLONE_CONFIG_<NAME>_TYPE` in the shared process environment).
    async fn configured(rc: &Rclone) -> Vec<(String, String)> {
        let mut v: Vec<(String, String)> = list_remotes(rc)
            .await
            .unwrap()
            .into_iter()
            .filter(|r| std::env::var(format!("RCLONE_CONFIG_{}_TYPE", r.name.trim_end_matches(':').to_ascii_uppercase())).is_err())
            .map(|r| (r.name, r.backend))
            .collect();
        v.sort();
        v
    }

    fn params(kv: &[(&str, &str)]) -> BTreeMap<String, String> {
        kv.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn parses_captured_provider_schemas() {
        let list = parse_providers(PROVIDERS_FIXTURE).unwrap();
        let names: Vec<&str> = list.iter().map(|p| p.name.as_str()).collect();
        assert!(names.windows(2).all(|w| w[0] <= w[1]), "sorted");
        for n in ["alias", "crypt", "drive", "google photos", "local", "s3"] {
            assert!(names.contains(&n), "{n} in {names:?}");
        }
        let crypt = find_provider(&list, "crypt").unwrap();
        let pw = crypt.options.iter().find(|o| o.name == "password").unwrap();
        assert!(pw.is_password && pw.required && pw.sensitive && !pw.advanced);
        let fe = crypt.options.iter().find(|o| o.name == "filename_encryption").unwrap();
        assert_eq!(fe.default, "standard");
        assert!(fe.examples.iter().any(|e| e.value == "off"));
        // Prefix lookup and the stored type name.
        assert_eq!(find_provider(&list, "gphotos").unwrap().name, "google photos");
        // Non-obscured secrets are flagged sensitive; advanced flags carried.
        let drive = find_provider(&list, "drive").unwrap();
        let cs = drive.options.iter().find(|o| o.name == "client_secret").unwrap();
        assert!(cs.sensitive && !cs.is_password);
        assert!(drive.options.iter().any(|o| o.advanced));
        assert!(drive.options.iter().find(|o| o.name == "token").unwrap().sensitive);
        // Provider-filtered options keep their filter; bool defaults become text.
        let s3 = find_provider(&list, "s3").unwrap();
        assert!(s3.options.iter().any(|o| o.name == "region" && o.provider.starts_with('!')));
        assert_eq!(s3.options.iter().find(|o| o.name == "env_auth").unwrap().default, "false");
        assert_eq!(s3.options.iter().find(|o| o.name == "env_auth").unwrap().option_type, "bool");
        assert!(parse_providers("nope").is_err());
    }

    #[test]
    fn steps_names_and_params() {
        let s = parse_step("NOTICE: x\n{\"State\":\"\",\"Option\":null,\"Error\":\"\",\"Result\":\"\"}").unwrap();
        assert!(s.done && s.option.is_none());
        let q = parse_step(r#"{"State":"*oauth-islocal,teamdrive,,","Option":{"Name":"config_is_local","Help":"Use auto config?","Default":true,"Examples":[{"Value":"true","Help":"Yes"},{"Value":"false","Help":"No"}],"Required":false,"IsPassword":false,"Type":"bool","Exclusive":true},"Error":"","Result":""}"#).unwrap();
        assert!(!q.done && is_oauth_question(&q));
        assert_eq!(q.option.as_ref().unwrap().default, "true");
        assert!(parse_step("garbage").is_err());

        assert_eq!(validate_name("my-drive:").unwrap(), "my-drive");
        assert_eq!(validate_name("Team Drive.2@x").unwrap(), "Team Drive.2@x");
        assert_eq!(validate_name(" padded ").unwrap(), "padded");
        for bad in ["", "-x", " -x", "a/b", "a:b", "a=b"] {
            assert!(validate_name(bad).is_err(), "{bad}");
        }
        assert_eq!(param_args(&params(&[("remote", "/a=b"), ("password", "-p w")])).unwrap(), vec!["password=-p w", "remote=/a=b"]);
        assert!(param_args(&params(&[("--config", "x")])).is_err());
        assert!(param_args(&params(&[("a", "line\nbreak")])).is_err());
        let a = continue_args("gd", "*oauth,x", "-weird", &params(&[("scope", "drive")]), ObscureMode::Auto).unwrap();
        assert_eq!(a, vec!["config", "update", "gd", "scope=drive", "--continue", "--non-interactive", "--state=*oauth,x", "--result=-weird"]);
        // Follow-up steps carry the create/update call's obscure mode.
        let a = continue_args("sec", "st", "", &params(&[("password", "pw")]), ObscureMode::Obscure).unwrap();
        assert_eq!(a, vec!["config", "update", "sec", "password=pw", "--continue", "--non-interactive", "--state=st", "--result=", "--obscure"]);
        let a = continue_args("sec", "st", "", &params(&[("password", "x6xL")]), ObscureMode::AlreadyObscured).unwrap();
        assert_eq!(a.last().map(String::as_str), Some("--no-obscure"));
        assert_eq!(ObscureMode::Obscure.flag(), Some("--obscure"));
        assert_eq!(ObscureMode::AlreadyObscured.flag(), Some("--no-obscure"));
    }

    #[test]
    fn redacts_secrets_from_details() {
        let list = parse_providers(PROVIDERS_FIXTURE).unwrap();
        let section: BTreeMap<String, String> = [("type", "crypt"), ("remote", "/data"), ("password", "obscured"), ("password2", ""), ("filename_encryption", "off")].iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
        let d = redact("sec", &section, find_provider(&list, "crypt"));
        assert_eq!(d.remote_type, "crypt");
        assert_eq!(d.secrets_set, vec!["password"]);
        assert!(!d.values.contains_key("password") && !d.values.contains_key("password2"));
        assert_eq!(d.values["remote"], "/data");
        // Without a schema, well-known secret names are still withheld.
        let s: BTreeMap<String, String> = [("type", "drive"), ("token", "{\"access_token\":\"x\"}"), ("client_id", "id")].iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
        let d = redact("gd", &s, None);
        assert_eq!((d.secrets_set.clone(), d.values.keys().cloned().collect::<Vec<_>>()), (vec!["token".to_string()], vec!["client_id".to_string()]));
        // …including the obscured `pass` of sftp/ftp/webdav/smb (reversible with `rclone reveal`).
        let s: BTreeMap<String, String> = [("type", "sftp"), ("host", "h"), ("user", "u"), ("pass", "x6xLnMK3"), ("key_file_pass", "abc")].iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
        let d = redact("box", &s, None);
        assert_eq!(d.secrets_set, vec!["key_file_pass", "pass"]);
        assert_eq!(d.values.keys().cloned().collect::<Vec<_>>(), vec!["host", "user"]);
    }

    #[test]
    fn oauth_helpers() {
        let help = "Execute the following on the machine with the web browser (same rclone\nversion recommended):\n\n\trclone authorize \"google photos\" \"eyJzY29wZSI6ImRyaXZlIn0\"\n\nThen paste the result.\n";
        assert_eq!(authorize_args(help).unwrap(), vec!["google photos", "eyJzY29wZSI6ImRyaXZlIn0"]);
        assert!(authorize_args("no command here").is_none());
        assert!(authorize_args("\trclone authorize --evil").is_none());
        assert_eq!(auth_url("2026/10/24 NOTICE: Please go to the following link: http://127.0.0.1:53682/auth?state=abc").as_deref(), Some("http://127.0.0.1:53682/auth?state=abc"));
        assert_eq!(auth_url("NOTICE: If your browser doesn't open automatically go to the following link: http://127.0.0.1:53682/auth?state=Q8"), Some("http://127.0.0.1:53682/auth?state=Q8".into()));
        assert_eq!(auth_url("NOTICE: Waiting for code..."), None);
        assert_eq!(authorize_token("Paste the following into your remote machine --->\n{\"access_token\":\"a\"}\n<---End paste\n").as_deref(), Some("{\"access_token\":\"a\"}"));
        assert_eq!(authorize_token("noise\n{\"access_token\":\"b\"}\n").as_deref(), Some("{\"access_token\":\"b\"}"));
        assert_eq!(authorize_token("nothing"), None);
    }

    /// The OAuth flow against a stand-in rclone that plays rclone's part:
    /// headless question, `authorize` printing the link then the token,
    /// and the final continuation receiving that token.
    #[cfg(unix)]
    #[tokio::test]
    async fn oauth_sign_in_runs_authorize_and_hands_back_the_token() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let log = d.path().join("calls.log");
        let fake = d.path().join("rclone");
        std::fs::write(
            &fake,
            format!(
                r#"#!/bin/sh
echo "$*" >> {log}
case "$*" in
  *listremotes*) echo "gd: drive" ;;
  *"--result=false"*) printf '%s\n' '{{"State":"*oauth-authorize,teamdrive,,","Option":{{"Name":"config_token","Help":"Execute:\n\n\trclone authorize \"drive\" \"eyJ4Ijoi\"\n\nThen paste the result.","Required":true,"Type":"string"}},"Error":"","Result":""}}' ;;
  *'--result={{"access_token":"tok"}}'*) printf '%s\n' '{{"State":"","Option":null,"Error":"","Result":""}}' ;;
  *" authorize "*) echo "2026/10/24 13:00:00 NOTICE: Please go to the following link: http://127.0.0.1:53682/auth?state=XYZ" >&2; sleep 1; printf 'Paste the following into your remote machine --->\n{{"access_token":"tok"}}\n<---End paste\n' ;;
  *) echo "unexpected: $*" >&2; exit 2 ;;
esac
"#,
                log = log.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let rc = Rclone::new(fake.to_string_lossy(), Some("/tmp/x.conf".into()));
        let urls = Arc::new(Mutex::new(Vec::new()));
        let u2 = urls.clone();
        let step = oauth_sign_in(&rc, "gd", "*oauth-islocal,teamdrive,,", &params(&[("scope", "drive")]), ObscureMode::Auto, &move |u| u2.lock().unwrap().push(u)).await.unwrap();
        assert!(step.done, "{step:?}");
        assert_eq!(*urls.lock().unwrap(), vec!["http://127.0.0.1:53682/auth?state=XYZ".to_string()]);
        let calls = std::fs::read_to_string(&log).unwrap();
        assert!(calls.contains("--config /tmp/x.conf authorize drive eyJ4Ijoi --auth-no-open-browser"), "{calls}");
        assert!(calls.contains("scope=drive --continue --non-interactive --state=*oauth-authorize,teamdrive,, --result={\"access_token\":\"tok\"}"), "{calls}");
        assert!(!cancel_sign_in("gd"), "finished flows are forgotten");

        // A cancelled sign-in stops authorize and reports it.
        let rc2 = rc.clone();
        let task = tokio::spawn(async move { oauth_sign_in(&rc2, "gd", "*oauth-islocal,teamdrive,,", &BTreeMap::new(), ObscureMode::Auto, &|_| {}).await });
        for _ in 0..100 {
            if cancel_sign_in("gd") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let e = task.await.unwrap().unwrap_err().user_message();
        assert!(e.contains("cancelled"), "{e}");
    }

    /// A second sign-in for the same remote supersedes the first (which
    /// stops, freeing rclone's callback port); the first one finishing must
    /// not forget the second, so Cancel still reaches it.
    #[cfg(unix)]
    #[tokio::test]
    async fn second_sign_in_for_a_name_stays_cancellable() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let fake = d.path().join("rclone");
        std::fs::write(
            &fake,
            r#"#!/bin/sh
case "$*" in
  *listremotes*) echo "gd2: drive" ;;
  *"--result=false"*) printf '%s\n' '{"State":"*oauth-authorize,,,","Option":{"Name":"config_token","Help":"\trclone authorize \"drive\"","Required":true,"Type":"string"},"Error":"","Result":""}' ;;
  *" authorize "*) echo "NOTICE: Please go to the following link: http://127.0.0.1:53682/auth?state=S" >&2; exec sleep 30 ;;
  *) exit 2 ;;
esac
"#,
        )
        .unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let rc = Rclone::new(fake.to_string_lossy(), Some("/tmp/x.conf".into()));
        let start = |seen: Arc<tokio::sync::Notify>| {
            let rc = rc.clone();
            tokio::spawn(async move { oauth_sign_in(&rc, "gd2", "*oauth-islocal,,,", &BTreeMap::new(), ObscureMode::Auto, &move |_| seen.notify_one()).await })
        };
        let (seen_a, seen_b) = (Arc::new(tokio::sync::Notify::new()), Arc::new(tokio::sync::Notify::new()));
        let a = start(seen_a.clone());
        tokio::time::timeout(Duration::from_secs(10), seen_a.notified()).await.unwrap();
        let b = start(seen_b.clone());
        tokio::time::timeout(Duration::from_secs(10), seen_b.notified()).await.unwrap();
        let ea = tokio::time::timeout(Duration::from_secs(5), a).await.expect("the first sign-in is superseded").unwrap().unwrap_err().user_message();
        assert!(ea.contains("cancelled"), "{ea}");
        assert!(cancel_sign_in("gd2"), "the second sign-in is still tracked");
        let eb = tokio::time::timeout(Duration::from_secs(5), b).await.unwrap().unwrap().unwrap_err().user_message();
        assert!(eb.contains("cancelled"), "{eb}");
        assert!(!cancel_sign_in("gd2"));
    }

    /// Real rclone `authorize`: the link is captured from its output and the
    /// process is stopped on cancel (no account needed).
    #[tokio::test]
    async fn real_authorize_prints_a_link_we_capture() {
        let Some((_d, rc)) = temp_rclone() else { return };
        let cancel = CancellationToken::new();
        let c2 = cancel.clone();
        let got = Arc::new(Mutex::new(None));
        let g2 = got.clone();
        let on_url = move |u: String| {
            *g2.lock().unwrap() = Some(u);
            c2.cancel();
        };
        let r = authorize(&rc, &["drive".to_string()], cancel, Duration::from_secs(20), &on_url).await;
        let seen = got.lock().unwrap().clone();
        match seen {
            Some(u) => {
                assert!(u.starts_with("http://127.0.0.1:") && u.contains("/auth?state="), "{u}");
                assert!(r.unwrap_err().user_message().contains("cancelled"));
            }
            // The local callback port is busy on this machine: rclone fails fast instead.
            None => assert!(r.is_err()),
        }
    }

    /// Follow-up steps re-send the original params (plain secrets included):
    /// they must keep the create call's obscure mode, or rclone stores a
    /// base64-looking password (≥ 22 chars) raw over the obscured value.
    #[tokio::test]
    async fn follow_up_steps_keep_passwords_obscured_with_real_rclone() {
        let Some((d, rc)) = temp_rclone() else { return };
        let pw = "abcdefghijklmnopqrstuvwxyz012345";
        let p = params(&[("remote", d.path().join("x").to_str().unwrap()), ("password", pw)]);
        assert!(create(&rc, "sec", "crypt", &p, ObscureMode::Obscure).await.unwrap().done);
        let stored = || {
            let conf = std::fs::read_to_string(rc.config.as_ref().unwrap()).unwrap();
            conf.lines().find_map(|l| l.strip_prefix("password = ")).unwrap().to_string()
        };
        assert_ne!(stored(), pw);
        let s = continue_with(&rc, "sec", "", "", &p, ObscureMode::Obscure).await.unwrap();
        assert!(s.done, "{s:?}");
        let v = stored();
        assert_ne!(v, pw, "password stored in plain text by the follow-up step");
        let revealed = rc.run(&["reveal".into(), v], QUICK).await.unwrap();
        assert_eq!(revealed.trim(), pw);
    }

    /// Create / update / list / details / test / delete with the real rclone
    /// binary and a temp config: local, alias and crypt remotes.
    #[tokio::test]
    async fn remote_lifecycle_with_real_rclone() {
        let Some((d, rc)) = temp_rclone() else { return };
        let data = d.path().join("data");
        std::fs::create_dir_all(data.join("sub")).unwrap();
        std::fs::write(data.join("a.txt"), "x").unwrap();

        // local
        let s = create(&rc, "loc", "local", &BTreeMap::new(), ObscureMode::Auto).await.unwrap();
        assert!(s.done, "{s:?}");
        // alias over a local folder
        let s = create(&rc, "al", "alias", &params(&[("remote", data.to_str().unwrap())]), ObscureMode::Auto).await.unwrap();
        assert!(s.done);
        let t = test_remote(&rc, "al").await.unwrap();
        assert!(t.ok && t.entries == 2, "{t:?}");
        // crypt over the alias, plain password obscured by rclone
        let s = create(&rc, "sec", "crypt", &params(&[("remote", "al:vault"), ("password", "hunter2")]), ObscureMode::Obscure).await.unwrap();
        assert!(s.done);
        assert_eq!(configured(&rc).await, vec![("al:".into(), "alias".into()), ("loc:".into(), "local".into()), ("sec:".into(), "crypt".into())]);
        let conf = std::fs::read_to_string(rc.config.as_ref().unwrap()).unwrap();
        assert!(!conf.contains("hunter2"), "password stored obscured");
        let det = remote_details(&rc, "sec").await.unwrap();
        assert_eq!((det.remote_type.as_str(), det.values["remote"].as_str()), ("crypt", "al:vault"));
        assert_eq!(det.secrets_set, vec!["password"]);
        assert!(!serde_json::to_string(&det).unwrap().contains(conf.lines().find(|l| l.starts_with("password")).unwrap().trim_start_matches("password = ")));

        // Duplicate names and unknown backends are refused.
        assert!(create(&rc, "loc", "local", &BTreeMap::new(), ObscureMode::Auto).await.unwrap_err().user_message().contains("already exists"));
        assert!(create(&rc, "x", "no-such-backend", &BTreeMap::new(), ObscureMode::Auto).await.unwrap_err().user_message().contains("no \"no-such-backend\""));

        // update
        let s = update(&rc, "al", &params(&[("remote", data.join("sub").to_str().unwrap())]), ObscureMode::Auto).await.unwrap();
        assert!(s.done);
        assert_eq!(remote_details(&rc, "al").await.unwrap().values["remote"], data.join("sub").to_string_lossy());
        assert_eq!(test_remote(&rc, "al").await.unwrap().entries, 0);
        assert!(update(&rc, "ghost", &BTreeMap::new(), ObscureMode::Auto).await.unwrap_err().user_message().contains("No rclone remote"));

        // A failing remote reports instead of erroring.
        create(&rc, "broken", "alias", &params(&[("remote", "nosuch:")]), ObscureMode::Auto).await.unwrap();
        let t = test_remote(&rc, "broken").await.unwrap();
        assert!(!t.ok && t.error.is_some());

        // delete
        delete(&rc, "broken").await.unwrap();
        delete(&rc, "loc").await.unwrap();
        let left: Vec<String> = configured(&rc).await.into_iter().map(|r| r.0).collect();
        assert_eq!(left, vec!["al:", "sec:"]);
        assert!(delete(&rc, "loc").await.is_err());

        // An OAuth backend asks the auto-config question first.
        let s = create(&rc, "gp", "google photos", &BTreeMap::new(), ObscureMode::Auto).await.unwrap();
        assert!(is_oauth_question(&s), "{s:?}");
        // Answering "headless" gets the token question naming `rclone authorize`.
        let s = continue_with(&rc, "gp", &s.state, "false", &BTreeMap::new(), ObscureMode::Auto).await.unwrap();
        assert_eq!(s.option.as_ref().map(|o| o.name.as_str()), Some("config_token"));
        assert_eq!(authorize_args(&s.option.unwrap().help).unwrap(), vec!["google photos"]);
    }
}
