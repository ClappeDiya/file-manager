//! Fleet policy templates → managed backup jobs.
//!
//! A policy `fpl_x` becomes the job `policy-fpl_x`. Central credentials
//! are referenced as `central:<id>`: a destination `credential_id` keeps
//! the reference (the secret store holds the value under that id), every
//! other string becomes the secret placeholder `$central:<id>`, resolved
//! from the secure store at run time. The template's
//! `passphraseCredentialId` turns into the stored passphrase
//! `$central:<id>`, so no secret value ever lands in the job configuration.

use crate::error::AppError;
use serde_json::{Map, Value};

pub const MANAGED_PREFIX: &str = "policy-";
pub const CENTRAL_PREFIX: &str = "central:";

fn valid_id(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Job id of the managed job for a policy.
pub fn managed_job_id(policy_id: &str) -> Result<String, AppError> {
    if !valid_id(policy_id) {
        return Err(AppError::validation(format!("Invalid policy id '{policy_id}'")));
    }
    Ok(format!("{MANAGED_PREFIX}{policy_id}"))
}

/// The policy id of a managed job id.
pub fn policy_of_job(job_id: &str) -> Option<&str> {
    job_id.strip_prefix(MANAGED_PREFIX).filter(|p| valid_id(p))
}

/// Normalize a central credential id (`central:<id>` or `<id>`); `None` if malformed.
pub fn central_credential_id(raw: &str) -> Option<String> {
    let id = raw.strip_prefix(CENTRAL_PREFIX).unwrap_or(raw);
    valid_id(id).then(|| format!("{CENTRAL_PREFIX}{id}"))
}

/// A template ready to overlay on a job.
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedTemplate {
    /// Job fields (snake_case, as `BackupJob` serializes), references converted.
    pub fields: Map<String, Value>,
    /// The passphrase to store: `$central:<id>` when the policy names one.
    pub passphrase: Option<String>,
    /// Central credentials the template references.
    pub credentials: Vec<String>,
}

fn convert(v: &mut Value, key: Option<&str>, refs: &mut Vec<String>) {
    match v {
        Value::String(s) if s.starts_with(CENTRAL_PREFIX) => {
            if let Some(id) = central_credential_id(s) {
                if !refs.contains(&id) {
                    refs.push(id.clone());
                }
                *s = if key == Some("credential_id") { id } else { format!("${id}") };
            }
        }
        Value::Array(a) => a.iter_mut().for_each(|x| convert(x, None, refs)),
        Value::Object(o) => o.iter_mut().for_each(|(k, x)| convert(x, Some(k.as_str()), refs)),
        _ => {}
    }
}

/// Validate a policy's job template and convert its credential references.
pub fn prepare_template(job: &Value) -> Result<PreparedTemplate, AppError> {
    let Some(obj) = job.as_object() else {
        return Err(AppError::validation("The policy's job template is not an object"));
    };
    let mut fields = obj.clone();
    let mut credentials = Vec::new();
    let passphrase = match fields.remove("passphraseCredentialId") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if s.trim().is_empty() => None,
        Some(Value::String(s)) => {
            let id = central_credential_id(&s).ok_or_else(|| AppError::validation(format!("Invalid passphrase credential '{s}'")))?;
            credentials.push(id.clone());
            Some(format!("${id}"))
        }
        Some(_) => return Err(AppError::validation("passphraseCredentialId must be a string")),
    };
    // Identity and statistics belong to the agent.
    for k in ["id", "stats", "created_at", "updated_at"] {
        fields.remove(k);
    }
    for (k, v) in fields.iter_mut() {
        convert(v, Some(k.as_str()), &mut credentials);
    }
    Ok(PreparedTemplate { fields, passphrase, credentials })
}

/// Overlay template fields on a job's JSON (top-level keys replace).
pub fn overlay(base: &mut Value, fields: &Map<String, Value>) {
    if let Some(o) = base.as_object_mut() {
        for (k, v) in fields {
            o.insert(k.clone(), v.clone());
        }
    }
}
