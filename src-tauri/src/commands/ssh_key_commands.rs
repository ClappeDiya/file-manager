//! SSH key generation commands.
//!
//! Provides key generation for SFTP authentication.
//! Supports Ed25519 (default) and RSA-4096 algorithms.

use crate::core::error::AppError;
use std::path::PathBuf;

/// Supported SSH key algorithms.
#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SshKeyAlgorithm {
    Ed25519,
    Rsa4096,
}

impl SshKeyAlgorithm {
    fn keygen_args(&self) -> Vec<&'static str> {
        match self {
            SshKeyAlgorithm::Ed25519 => vec!["-t", "ed25519"],
            SshKeyAlgorithm::Rsa4096 => vec!["-t", "rsa", "-b", "4096"],
        }
    }

    fn default_filename(&self) -> &'static str {
        match self {
            SshKeyAlgorithm::Ed25519 => "id_ed25519_ufop",
            SshKeyAlgorithm::Rsa4096 => "id_rsa_ufop",
        }
    }
}

/// Result of SSH key generation.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SshKeyGenResult {
    /// Path to the private key file.
    pub private_key_path: String,
    /// Path to the public key file.
    pub public_key_path: String,
    /// The public key content (for display/copy).
    pub public_key_content: String,
    /// Algorithm used.
    pub algorithm: String,
}

/// Generate a new SSH key pair.
///
/// Uses the system `ssh-keygen` binary for maximum compatibility and security.
#[tauri::command]
pub async fn generate_ssh_key(
    algorithm: Option<String>,
    passphrase: Option<String>,
    path: Option<String>,
    comment: Option<String>,
) -> Result<SshKeyGenResult, AppError> {
    let algo = match algorithm.as_deref() {
        Some("rsa4096") | Some("rsa") => SshKeyAlgorithm::Rsa4096,
        _ => SshKeyAlgorithm::Ed25519,
    };

    // Determine output path
    let ssh_dir = dirs::home_dir()
        .ok_or_else(|| AppError::Sync {
            message: "Cannot determine home directory".to_string(),
            advice: "Check your system configuration.".to_string(),
        })?
        .join(".ssh");

    // Ensure .ssh directory exists with correct permissions
    if !ssh_dir.exists() {
        std::fs::create_dir_all(&ssh_dir).map_err(|e| AppError::Sync {
            message: format!("Failed to create ~/.ssh directory: {e}"),
            advice: "Check filesystem permissions.".to_string(),
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&ssh_dir, std::fs::Permissions::from_mode(0o700))
                .map_err(|e| AppError::Sync {
                    message: format!("Failed to set permissions on ~/.ssh: {e}"),
                    advice: "Check filesystem permissions.".to_string(),
                })?;
        }
    }

    let key_path = if let Some(p) = path {
        PathBuf::from(p)
    } else {
        ssh_dir.join(algo.default_filename())
    };

    // Check if key already exists
    if key_path.exists() {
        return Err(AppError::Sync {
            message: format!("Key already exists at {}", key_path.display()),
            advice: "Choose a different path or delete the existing key.".to_string(),
        });
    }

    // Build ssh-keygen command
    let mut cmd = std::process::Command::new("ssh-keygen");
    cmd.args(algo.keygen_args());
    cmd.arg("-f").arg(&key_path);

    // Comment
    let key_comment = comment.unwrap_or_else(|| {
        format!(
            "ufop@{}",
            hostname::get()
                .map(|h| h.to_string_lossy().to_string())
                .unwrap_or_else(|_| "unknown".to_string())
        )
    });
    cmd.arg("-C").arg(&key_comment);

    // Passphrase: use SSH_ASKPASS with a temp script to avoid passing
    // the passphrase via command-line argument (visible in `ps aux`).
    let pass = passphrase.unwrap_or_default();

    if pass.is_empty() {
        // No passphrase — pass empty string via -N (safe, no secret to leak)
        cmd.arg("-N").arg("");
    } else {
        // Create a temporary askpass script that echoes the passphrase
        let askpass_dir = std::env::temp_dir();
        let askpass_path = askpass_dir.join(format!("ufop_askpass_{}.sh", std::process::id()));
        // Write script: #!/bin/sh\necho '<escaped_pass>'
        let escaped_pass = pass.replace('\'', "'\\''");
        std::fs::write(&askpass_path, format!("#!/bin/sh\necho '{escaped_pass}'"))
            .map_err(|e| AppError::Sync {
                message: format!("Failed to create askpass helper: {e}"),
                advice: "Check temp directory permissions.".to_string(),
            })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&askpass_path, std::fs::Permissions::from_mode(0o700))
                .map_err(|e| AppError::Sync {
                    message: format!("Failed to set askpass permissions: {e}"),
                    advice: "Check temp directory permissions.".to_string(),
                })?;
        }
        cmd.arg("-N").arg(""); // Trigger passphrase prompt
        cmd.env("SSH_ASKPASS", &askpass_path);
        cmd.env("SSH_ASKPASS_REQUIRE", "force");
        cmd.env("DISPLAY", ":0"); // Required to trigger SSH_ASKPASS

        // ssh-keygen needs no controlling terminal to use SSH_ASKPASS
        cmd.stdin(std::process::Stdio::null());

        let output = cmd.output().map_err(|e| AppError::Sync {
            message: format!("Failed to run ssh-keygen: {e}"),
            advice: "Ensure ssh-keygen is installed and accessible in PATH.".to_string(),
        });

        // Clean up askpass script immediately
        let _ = std::fs::remove_file(&askpass_path);

        let output = output?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(AppError::Sync {
                message: format!("ssh-keygen failed: {stderr}"),
                advice: "Check the key type, path, and passphrase.".to_string(),
            });
        }

        // Set correct permissions on the private key
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600));
        }

        // Read the generated public key
        let pub_key_path = format!("{}.pub", key_path.display());
        let public_key = std::fs::read_to_string(&pub_key_path).unwrap_or_default();

        return Ok(SshKeyGenResult {
            private_key_path: key_path.to_string_lossy().to_string(),
            public_key_path: pub_key_path,
            public_key_content: public_key,
            algorithm: format!("{:?}", algo),
        });
    }

    // Run ssh-keygen (no passphrase case)
    let output = cmd.output().map_err(|e| AppError::Sync {
        message: format!("Failed to run ssh-keygen: {e}"),
        advice: "Ensure ssh-keygen is installed and accessible in PATH.".to_string(),
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(AppError::Sync {
            message: format!("ssh-keygen failed: {stderr}"),
            advice: "Check the error message above.".to_string(),
        });
    }

    // Set correct permissions on the private key
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| AppError::Sync {
                message: format!("Failed to set key permissions: {e}"),
                advice: "Check filesystem permissions.".to_string(),
            })?;
    }

    // Read the public key content
    let pub_key_path = PathBuf::from(format!("{}.pub", key_path.display()));
    let public_key_content = std::fs::read_to_string(&pub_key_path).map_err(|e| AppError::Sync {
        message: format!("Failed to read public key: {e}"),
        advice: "Key generation may have partially failed.".to_string(),
    })?;

    Ok(SshKeyGenResult {
        private_key_path: key_path.to_string_lossy().to_string(),
        public_key_path: pub_key_path.to_string_lossy().to_string(),
        public_key_content: public_key_content.trim().to_string(),
        algorithm: match algo {
            SshKeyAlgorithm::Ed25519 => "ed25519".to_string(),
            SshKeyAlgorithm::Rsa4096 => "rsa-4096".to_string(),
        },
    })
}

/// List existing SSH keys in ~/.ssh/.
#[tauri::command]
pub async fn list_ssh_keys() -> Result<Vec<SshKeyInfo>, AppError> {
    let ssh_dir = dirs::home_dir()
        .ok_or_else(|| AppError::Sync {
            message: "Cannot determine home directory".to_string(),
            advice: "Check your system configuration.".to_string(),
        })?
        .join(".ssh");

    if !ssh_dir.exists() {
        return Ok(Vec::new());
    }

    let mut keys = Vec::new();
    let entries = std::fs::read_dir(&ssh_dir).map_err(|e| AppError::Sync {
        message: format!("Failed to read ~/.ssh: {e}"),
        advice: "Check directory permissions.".to_string(),
    })?;

    for entry in entries {
        let entry = entry.map_err(|e| AppError::Sync {
            message: format!("Failed to read directory entry: {e}"),
            advice: "Check file permissions.".to_string(),
        })?;

        let path = entry.path();
        let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();

        // Look for private key files (matching common patterns, no .pub extension)
        if path.is_file()
            && !name.ends_with(".pub")
            && !name.starts_with("known_hosts")
            && !name.starts_with("authorized_keys")
            && !name.starts_with("config")
            && !name.starts_with(".")
        {
            // Check if there's a corresponding .pub file
            let pub_path = PathBuf::from(format!("{}.pub", path.display()));
            if pub_path.exists() {
                let algorithm = if name.contains("ed25519") {
                    "ed25519"
                } else if name.contains("ecdsa") {
                    "ecdsa"
                } else if name.contains("rsa") {
                    "rsa"
                } else if name.contains("dsa") {
                    "dsa"
                } else {
                    "unknown"
                };

                keys.push(SshKeyInfo {
                    name: name.clone(),
                    path: path.to_string_lossy().to_string(),
                    public_key_path: pub_path.to_string_lossy().to_string(),
                    algorithm: algorithm.to_string(),
                });
            }
        }
    }

    Ok(keys)
}

/// Information about an existing SSH key.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SshKeyInfo {
    pub name: String,
    pub path: String,
    pub public_key_path: String,
    pub algorithm: String,
}
