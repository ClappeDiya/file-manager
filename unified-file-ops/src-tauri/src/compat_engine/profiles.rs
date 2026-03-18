//! Destination profiles and cloud rules (T-035, T-036).
//!
//! Defines 12+ profiles covering local filesystems, cloud services,
//! and transport layers. Each profile specifies:
//! - Forbidden characters
//! - Reserved names
//! - Maximum path/filename lengths
//! - Case sensitivity
//! - Unicode normalization preference
//! - Trailing character rules
//!
//! Also implements:
//! - Reserved name detection/escaping (CON, PRN, AUX, NUL, COM1-9, LPT1-9)
//! - Path length validation per profile
//! - Case collision detection for case-insensitive targets
//! - CJK 3-byte UTF-8 awareness for ext4 255-byte limit

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Case sensitivity mode for a filesystem profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaseSensitivity {
    /// Fully case-sensitive (ext4, APFS-CS)
    Sensitive,
    /// Case-insensitive but case-preserving (NTFS, APFS-CI, SMB, most cloud)
    Insensitive,
}

/// A destination profile describing filesystem constraints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DestProfile {
    /// Unique profile identifier.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Characters forbidden in filenames.
    pub forbidden_chars: Vec<char>,
    /// Reserved names (case-insensitive match).
    pub reserved_names: Vec<String>,
    /// Maximum full path length in bytes.
    pub max_path_bytes: usize,
    /// Maximum single filename length in bytes.
    pub max_filename_bytes: usize,
    /// Case sensitivity mode.
    pub case_sensitivity: CaseSensitivity,
    /// Preferred Unicode normalization form.
    pub unicode_pref: String, // "NFC" or "NFD"
    /// Whether trailing dots are forbidden.
    pub no_trailing_dots: bool,
    /// Whether trailing spaces are forbidden.
    pub no_trailing_spaces: bool,
    /// Whether leading dots create hidden files (informational).
    pub leading_dot_hides: bool,
    /// Profile description.
    pub description: String,
    /// Parent profiles for chaining (e.g., SMB-to-Windows chains SMB + Windows).
    pub chain_profiles: Vec<String>,
}

/// Windows reserved device names.
pub const WINDOWS_RESERVED_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL",
    "COM0", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9",
    "LPT0", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Build all built-in destination profiles.
pub fn all_profiles() -> Vec<DestProfile> {
    vec![
        windows_ntfs_profile(),
        windows_fat32_profile(),
        macos_apfs_ci_profile(),
        macos_apfs_cs_profile(),
        ext4_profile(),
        onedrive_profile(),
        google_drive_profile(),
        dropbox_profile(),
        s3_profile(),
        backblaze_b2_profile(),
        smb_to_windows_profile(),
        strict_enterprise_profile(),
    ]
}

/// Get a profile by ID.
pub fn get_profile(id: &str) -> Option<DestProfile> {
    all_profiles().into_iter().find(|p| p.id == id)
}

/// Resolve a profile chain: returns all profiles in order (leaf first).
pub fn resolve_chain(profile_id: &str) -> Vec<DestProfile> {
    let mut result = Vec::new();
    let mut visited = HashSet::new();
    resolve_chain_inner(profile_id, &mut result, &mut visited);
    result
}

fn resolve_chain_inner(id: &str, result: &mut Vec<DestProfile>, visited: &mut HashSet<String>) {
    if visited.contains(id) {
        return; // prevent cycles
    }
    visited.insert(id.to_string());

    if let Some(profile) = get_profile(id) {
        // First add parent profiles
        for parent_id in &profile.chain_profiles {
            resolve_chain_inner(parent_id, result, visited);
        }
        result.push(profile);
    }
}

/// Merge constraints from a profile chain (most restrictive wins).
pub fn merge_profiles(profiles: &[DestProfile]) -> DestProfile {
    if profiles.is_empty() {
        return strict_enterprise_profile(); // fallback to strictest
    }
    if profiles.len() == 1 {
        return profiles[0].clone();
    }

    let first = &profiles[0];
    let mut merged = first.clone();
    merged.id = profiles.iter().map(|p| p.id.as_str()).collect::<Vec<_>>().join("+");
    merged.name = format!("Merged: {}", merged.id);

    for profile in &profiles[1..] {
        // Union of forbidden chars
        for &ch in &profile.forbidden_chars {
            if !merged.forbidden_chars.contains(&ch) {
                merged.forbidden_chars.push(ch);
            }
        }
        // Union of reserved names
        for name in &profile.reserved_names {
            let upper = name.to_uppercase();
            if !merged.reserved_names.iter().any(|n| n.to_uppercase() == upper) {
                merged.reserved_names.push(name.clone());
            }
        }
        // Smallest max path/filename
        merged.max_path_bytes = merged.max_path_bytes.min(profile.max_path_bytes);
        merged.max_filename_bytes = merged.max_filename_bytes.min(profile.max_filename_bytes);
        // Case insensitive if any is insensitive
        if profile.case_sensitivity == CaseSensitivity::Insensitive {
            merged.case_sensitivity = CaseSensitivity::Insensitive;
        }
        // Strictest trailing rules
        merged.no_trailing_dots = merged.no_trailing_dots || profile.no_trailing_dots;
        merged.no_trailing_spaces = merged.no_trailing_spaces || profile.no_trailing_spaces;
    }

    merged
}

// ── Profile definitions ──

fn windows_reserved() -> Vec<String> {
    WINDOWS_RESERVED_NAMES.iter().map(|s| s.to_string()).collect()
}

fn windows_forbidden() -> Vec<char> {
    vec!['<', '>', ':', '"', '|', '?', '*', '/', '\\']
}

fn windows_ntfs_profile() -> DestProfile {
    DestProfile {
        id: "windows-ntfs".to_string(),
        name: "Windows NTFS".to_string(),
        forbidden_chars: windows_forbidden(),
        reserved_names: windows_reserved(),
        max_path_bytes: 32767, // Extended path (\\?\)
        max_filename_bytes: 255,
        case_sensitivity: CaseSensitivity::Insensitive,
        unicode_pref: "NFC".to_string(),
        no_trailing_dots: true,
        no_trailing_spaces: true,
        leading_dot_hides: false,
        description: "Windows NTFS with extended path support".to_string(),
        chain_profiles: vec![],
    }
}

fn windows_fat32_profile() -> DestProfile {
    DestProfile {
        id: "windows-fat32".to_string(),
        name: "Windows FAT32".to_string(),
        forbidden_chars: windows_forbidden(),
        reserved_names: windows_reserved(),
        max_path_bytes: 260, // Classic MAX_PATH
        max_filename_bytes: 255,
        case_sensitivity: CaseSensitivity::Insensitive,
        unicode_pref: "NFC".to_string(),
        no_trailing_dots: true,
        no_trailing_spaces: true,
        leading_dot_hides: false,
        description: "Windows FAT32 with classic MAX_PATH limit".to_string(),
        chain_profiles: vec![],
    }
}

fn macos_apfs_ci_profile() -> DestProfile {
    DestProfile {
        id: "macos-apfs-ci".to_string(),
        name: "macOS APFS (Case-Insensitive)".to_string(),
        forbidden_chars: vec![':', '/'],
        reserved_names: vec![],
        max_path_bytes: 1024,
        max_filename_bytes: 255,
        case_sensitivity: CaseSensitivity::Insensitive,
        unicode_pref: "NFD".to_string(),
        no_trailing_dots: false,
        no_trailing_spaces: false,
        leading_dot_hides: true,
        description: "macOS APFS default (case-insensitive, case-preserving)".to_string(),
        chain_profiles: vec![],
    }
}

fn macos_apfs_cs_profile() -> DestProfile {
    DestProfile {
        id: "macos-apfs-cs".to_string(),
        name: "macOS APFS (Case-Sensitive)".to_string(),
        forbidden_chars: vec![':', '/'],
        reserved_names: vec![],
        max_path_bytes: 1024,
        max_filename_bytes: 255,
        case_sensitivity: CaseSensitivity::Sensitive,
        unicode_pref: "NFD".to_string(),
        no_trailing_dots: false,
        no_trailing_spaces: false,
        leading_dot_hides: true,
        description: "macOS APFS case-sensitive variant".to_string(),
        chain_profiles: vec![],
    }
}

fn ext4_profile() -> DestProfile {
    DestProfile {
        id: "ext4".to_string(),
        name: "Linux ext4".to_string(),
        forbidden_chars: vec!['/'],
        reserved_names: vec![],
        max_path_bytes: 4096,       // PATH_MAX
        max_filename_bytes: 255,    // NAME_MAX (bytes, not chars!)
        case_sensitivity: CaseSensitivity::Sensitive,
        unicode_pref: "NFC".to_string(),
        no_trailing_dots: false,
        no_trailing_spaces: false,
        leading_dot_hides: true,
        description: "Linux ext4 (255 bytes per filename component)".to_string(),
        chain_profiles: vec![],
    }
}

fn onedrive_profile() -> DestProfile {
    DestProfile {
        id: "onedrive".to_string(),
        name: "Microsoft OneDrive".to_string(),
        forbidden_chars: windows_forbidden(),
        reserved_names: windows_reserved(),
        max_path_bytes: 400,  // 400 decoded characters
        max_filename_bytes: 255,
        case_sensitivity: CaseSensitivity::Insensitive,
        unicode_pref: "NFC".to_string(),
        no_trailing_dots: true,
        no_trailing_spaces: true,
        leading_dot_hides: false,
        description: "OneDrive (400-char path limit, Windows naming rules)".to_string(),
        chain_profiles: vec![],
    }
}

fn google_drive_profile() -> DestProfile {
    DestProfile {
        id: "google-drive".to_string(),
        name: "Google Drive".to_string(),
        forbidden_chars: vec!['/'], // only / is truly forbidden
        reserved_names: vec![],
        max_path_bytes: 32767,
        max_filename_bytes: 255,
        case_sensitivity: CaseSensitivity::Insensitive,
        unicode_pref: "NFC".to_string(),
        no_trailing_dots: false,
        no_trailing_spaces: true, // trailing spaces silently stripped
        leading_dot_hides: false,
        description: "Google Drive (permissive, trailing spaces stripped)".to_string(),
        chain_profiles: vec![],
    }
}

fn dropbox_profile() -> DestProfile {
    DestProfile {
        id: "dropbox".to_string(),
        name: "Dropbox".to_string(),
        forbidden_chars: vec!['/', '\\'],
        reserved_names: windows_reserved(), // syncs to Windows clients
        max_path_bytes: 260, // Effectively Windows MAX_PATH for sync clients
        max_filename_bytes: 255,
        case_sensitivity: CaseSensitivity::Insensitive,
        unicode_pref: "NFC".to_string(),
        no_trailing_dots: true,
        no_trailing_spaces: true,
        leading_dot_hides: false,
        description: "Dropbox (Windows sync compatibility)".to_string(),
        chain_profiles: vec![],
    }
}

fn s3_profile() -> DestProfile {
    DestProfile {
        id: "s3".to_string(),
        name: "Amazon S3".to_string(),
        forbidden_chars: vec![], // S3 allows almost anything in keys
        reserved_names: vec![],
        max_path_bytes: 1024,   // 1024 bytes for key
        max_filename_bytes: 1024,
        case_sensitivity: CaseSensitivity::Sensitive,
        unicode_pref: "NFC".to_string(),
        no_trailing_dots: false,
        no_trailing_spaces: false,
        leading_dot_hides: false,
        description: "Amazon S3 (1024 byte key limit, nearly unrestricted)".to_string(),
        chain_profiles: vec![],
    }
}

fn backblaze_b2_profile() -> DestProfile {
    DestProfile {
        id: "backblaze-b2".to_string(),
        name: "Backblaze B2".to_string(),
        forbidden_chars: vec![],
        reserved_names: vec![],
        max_path_bytes: 1024,
        max_filename_bytes: 1024,
        case_sensitivity: CaseSensitivity::Sensitive,
        unicode_pref: "NFC".to_string(),
        no_trailing_dots: false,
        no_trailing_spaces: false,
        leading_dot_hides: false,
        description: "Backblaze B2 (S3-compatible, 1024 byte key limit)".to_string(),
        chain_profiles: vec![],
    }
}

fn smb_to_windows_profile() -> DestProfile {
    DestProfile {
        id: "smb-to-windows".to_string(),
        name: "SMB to Windows Target".to_string(),
        forbidden_chars: windows_forbidden(),
        reserved_names: windows_reserved(),
        max_path_bytes: 260, // SMB often limited to classic MAX_PATH
        max_filename_bytes: 255,
        case_sensitivity: CaseSensitivity::Insensitive,
        unicode_pref: "NFC".to_string(),
        no_trailing_dots: true,
        no_trailing_spaces: true,
        leading_dot_hides: false,
        description: "SMB share targeting Windows (transport + destination rules)".to_string(),
        chain_profiles: vec!["windows-ntfs".to_string()],
    }
}

fn strict_enterprise_profile() -> DestProfile {
    DestProfile {
        id: "strict-enterprise".to_string(),
        name: "Strict Enterprise".to_string(),
        forbidden_chars: {
            let mut chars = windows_forbidden();
            chars.push('#');
            chars.push('%');
            chars.push('&');
            chars.push('{');
            chars.push('}');
            chars.push('$');
            chars.push('!');
            chars.push('@');
            chars.push('\'');
            chars.push('`');
            chars.push('+');
            chars.push('=');
            chars
        },
        reserved_names: windows_reserved(),
        max_path_bytes: 200, // Conservative for deep nesting
        max_filename_bytes: 100, // Short names for compatibility
        case_sensitivity: CaseSensitivity::Insensitive,
        unicode_pref: "NFC".to_string(),
        no_trailing_dots: true,
        no_trailing_spaces: true,
        leading_dot_hides: false,
        description: "Strict enterprise policy (maximally restrictive)".to_string(),
        chain_profiles: vec![],
    }
}

// ── Reserved name handling (T-035) ──

/// Check if a filename is a Windows reserved name (case-insensitive).
/// Handles both bare names (CON) and names with extensions (CON.txt).
pub fn is_reserved_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or("").to_uppercase();
    WINDOWS_RESERVED_NAMES.contains(&stem.as_str())
}

/// Escape a reserved name by prefixing with underscore.
/// CON.txt -> _CON.txt
pub fn escape_reserved(name: &str) -> (String, Option<String>) {
    if is_reserved_name(name) {
        let escaped = format!("_{name}");
        let rule = format!("Prefixed reserved name '{}' with '_'",
            name.split('.').next().unwrap_or(name));
        (escaped, Some(rule))
    } else {
        (name.to_string(), None)
    }
}

/// Unescape a previously escaped reserved name.
/// _CON.txt -> CON.txt (only if the unescaped form is actually reserved)
pub fn unescape_reserved(name: &str) -> String {
    if let Some(candidate) = name.strip_prefix('_') {
        if is_reserved_name(candidate) {
            return candidate.to_string();
        }
    }
    name.to_string()
}

// ── Path length handling (T-035) ──

/// Result of a path length check.
#[derive(Debug, Clone)]
pub struct PathLengthResult {
    pub path: String,
    pub path_bytes: usize,
    pub filename_bytes: usize,
    pub max_path_bytes: usize,
    pub max_filename_bytes: usize,
    pub path_ok: bool,
    pub filename_ok: bool,
    pub shortened: Option<String>,
    pub rules: Vec<String>,
}

/// Check and optionally shorten a path to fit within profile limits.
pub fn check_path_length(path: &str, profile: &DestProfile) -> PathLengthResult {
    let path_bytes = path.len();
    let filename = path.rsplit('/').next().unwrap_or(path);
    let filename_bytes = filename.len();
    let path_ok = path_bytes <= profile.max_path_bytes;
    let filename_ok = filename_bytes <= profile.max_filename_bytes;
    let mut rules = Vec::new();

    let shortened = if !filename_ok {
        Some(shorten_filename(filename, profile.max_filename_bytes, &mut rules))
    } else if !path_ok {
        rules.push(format!(
            "Full path exceeds {}-byte limit ({} bytes)",
            profile.max_path_bytes, path_bytes
        ));
        None // Path shortening requires directory restructuring, which is beyond filename scope
    } else {
        None
    };

    PathLengthResult {
        path: path.to_string(),
        path_bytes,
        filename_bytes,
        max_path_bytes: profile.max_path_bytes,
        max_filename_bytes: profile.max_filename_bytes,
        path_ok,
        filename_ok,
        shortened,
        rules,
    }
}

/// Shorten a filename to fit within max_bytes, preserving extension and adding a hash suffix.
pub fn shorten_filename(name: &str, max_bytes: usize, rules: &mut Vec<String>) -> String {
    if name.len() <= max_bytes {
        return name.to_string();
    }

    // Split into stem and extension
    let (stem, ext) = if let Some(dot_pos) = name.rfind('.') {
        if dot_pos > 0 {
            (&name[..dot_pos], &name[dot_pos..]) // includes the dot
        } else {
            (name, "")
        }
    } else {
        (name, "")
    };

    // Generate 4-char hash suffix from original name
    let hash = simple_hash(name);
    let suffix = format!("~{hash}");

    // Calculate available space for stem
    let ext_bytes = ext.len();
    let suffix_bytes = suffix.len();
    let available = max_bytes.saturating_sub(ext_bytes + suffix_bytes);

    if available == 0 {
        // Extreme case: even the extension is too long
        rules.push(format!("Truncated filename to {max_bytes} bytes"));
        let mut result = name.to_string();
        truncate_to_bytes(&mut result, max_bytes);
        return result;
    }

    // Truncate stem at a valid UTF-8 boundary
    let mut truncated_stem = stem.to_string();
    truncate_to_bytes(&mut truncated_stem, available);

    let result = format!("{truncated_stem}{suffix}{ext}");
    rules.push(format!(
        "Shortened filename from {} to {} bytes (preserved extension '{}')",
        name.len(),
        result.len(),
        ext
    ));
    result
}

/// Truncate a string to fit within max_bytes at a valid UTF-8 char boundary.
fn truncate_to_bytes(s: &mut String, max_bytes: usize) {
    if s.len() <= max_bytes {
        return;
    }
    // Find the last valid char boundary at or before max_bytes
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s.truncate(end);
}

/// Simple 4-character hex hash for filename shortening.
fn simple_hash(s: &str) -> String {
    let mut h: u32 = 0;
    for byte in s.as_bytes() {
        h = h.wrapping_mul(31).wrapping_add(*byte as u32);
    }
    format!("{:04x}", h & 0xFFFF)
}

// ── Case collision detection (T-035) ──

/// Detect case collisions in a set of filenames for case-insensitive targets.
/// Returns groups of colliding filenames.
pub fn detect_case_collisions(names: &[&str]) -> Vec<Vec<String>> {
    let mut map: HashMap<String, Vec<String>> = HashMap::new();
    for &name in names {
        let key = name.to_lowercase();
        map.entry(key).or_default().push(name.to_string());
    }
    map.into_values()
        .filter(|group| group.len() > 1)
        .collect()
}

// ── ext4 byte-length awareness (T-035) ──

/// Check if a filename exceeds ext4's 255-byte limit.
/// CJK characters are 3 bytes in UTF-8, so a 255-char CJK filename
/// is actually 765 bytes -- well over the limit.
pub fn check_ext4_byte_length(name: &str) -> (bool, usize) {
    let byte_len = name.len();
    (byte_len <= 255, byte_len)
}

// ── Auto-detection (T-036) ──

/// Auto-detect the appropriate profile for a given filesystem path.
/// Returns the profile ID string.
pub fn auto_detect_profile(path: &str) -> String {
    let path_lower = path.to_lowercase();

    // Cloud service URLs/prefixes
    if path_lower.starts_with("s3://") || path_lower.contains("amazonaws.com") {
        return "s3".to_string();
    }
    if path_lower.starts_with("b2://") || path_lower.contains("backblazeb2.com") {
        return "backblaze-b2".to_string();
    }
    if path_lower.contains("onedrive") || path_lower.contains("sharepoint") {
        return "onedrive".to_string();
    }
    if path_lower.contains("google") && path_lower.contains("drive") {
        return "google-drive".to_string();
    }
    if path_lower.contains("dropbox") {
        return "dropbox".to_string();
    }

    // SMB paths
    if path_lower.starts_with("\\\\") || path_lower.starts_with("smb://") {
        return "smb-to-windows".to_string();
    }

    // Local filesystem detection by OS
    #[cfg(target_os = "windows")]
    {
        return "windows-ntfs".to_string();
    }

    #[cfg(target_os = "macos")]
    {
        "macos-apfs-ci".to_string()
    }

    #[cfg(target_os = "linux")]
    {
        return "ext4".to_string();
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        return "strict-enterprise".to_string();
    }
}

/// Detect profile from a connection protocol string.
pub fn profile_from_protocol(protocol: &str) -> String {
    match protocol {
        "s3" => "s3".to_string(),
        "onedrive" => "onedrive".to_string(),
        "google_drive" => "google-drive".to_string(),
        "dropbox" => "dropbox".to_string(),
        "smb" => "smb-to-windows".to_string(),
        "sftp" | "ftp" | "ftps" => "ext4".to_string(), // assume Linux remote
        "webdav" => "strict-enterprise".to_string(),    // safest default
        "local" => auto_detect_profile("/"),
        _ => "strict-enterprise".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Profile registry ──

    #[test]
    fn test_all_profiles_count() {
        let profiles = all_profiles();
        assert!(profiles.len() >= 12, "Expected 12+ profiles, got {}", profiles.len());
    }

    #[test]
    fn test_all_profiles_unique_ids() {
        let profiles = all_profiles();
        let mut ids: Vec<&str> = profiles.iter().map(|p| p.id.as_str()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), profiles.len(), "Duplicate profile IDs detected");
    }

    #[test]
    fn test_get_profile_exists() {
        assert!(get_profile("windows-ntfs").is_some());
        assert!(get_profile("ext4").is_some());
        assert!(get_profile("onedrive").is_some());
        assert!(get_profile("s3").is_some());
    }

    #[test]
    fn test_get_profile_not_exists() {
        assert!(get_profile("nonexistent").is_none());
    }

    // ── Profile chaining ──

    #[test]
    fn test_smb_to_windows_chain() {
        let chain = resolve_chain("smb-to-windows");
        assert!(chain.len() >= 2, "SMB-to-Windows should chain with windows-ntfs");
        let ids: Vec<&str> = chain.iter().map(|p| p.id.as_str()).collect();
        assert!(ids.contains(&"windows-ntfs"));
        assert!(ids.contains(&"smb-to-windows"));
    }

    #[test]
    fn test_no_chain_for_simple_profile() {
        let chain = resolve_chain("ext4");
        assert_eq!(chain.len(), 1);
    }

    #[test]
    fn test_merge_profiles() {
        let profiles = resolve_chain("smb-to-windows");
        let merged = merge_profiles(&profiles);
        // Merged should have all Windows forbidden chars
        assert!(merged.forbidden_chars.contains(&'<'));
        assert!(merged.forbidden_chars.contains(&'>'));
        // Should have reserved names
        assert!(!merged.reserved_names.is_empty());
        // Case insensitive since both are
        assert_eq!(merged.case_sensitivity, CaseSensitivity::Insensitive);
    }

    // ── Reserved names (T-035) ──

    #[test]
    fn test_reserved_name_bare() {
        assert!(is_reserved_name("CON"));
        assert!(is_reserved_name("con"));
        assert!(is_reserved_name("Con"));
        assert!(is_reserved_name("PRN"));
        assert!(is_reserved_name("AUX"));
        assert!(is_reserved_name("NUL"));
    }

    #[test]
    fn test_reserved_name_with_extension() {
        assert!(is_reserved_name("CON.txt"));
        assert!(is_reserved_name("con.txt"));
        assert!(is_reserved_name("PRN.doc"));
        assert!(is_reserved_name("NUL.log"));
    }

    #[test]
    fn test_reserved_name_com_lpt() {
        for i in 0..=9 {
            assert!(is_reserved_name(&format!("COM{i}")));
            assert!(is_reserved_name(&format!("LPT{i}")));
            assert!(is_reserved_name(&format!("com{i}.txt")));
            assert!(is_reserved_name(&format!("lpt{i}.doc")));
        }
    }

    #[test]
    fn test_not_reserved() {
        assert!(!is_reserved_name("CONTACT.txt"));
        assert!(!is_reserved_name("PRINTER.doc"));
        assert!(!is_reserved_name("auxiliary.log"));
        assert!(!is_reserved_name("null_value.txt"));
    }

    #[test]
    fn test_escape_reserved() {
        let (escaped, rule) = escape_reserved("CON.txt");
        assert_eq!(escaped, "_CON.txt");
        assert!(rule.is_some());
    }

    #[test]
    fn test_unescape_reserved() {
        let unescaped = unescape_reserved("_CON.txt");
        assert_eq!(unescaped, "CON.txt");
    }

    #[test]
    fn test_unescape_non_reserved() {
        let unescaped = unescape_reserved("_normal.txt");
        assert_eq!(unescaped, "_normal.txt"); // Not a reserved name, keep as-is
    }

    // ── Path length (T-035) ──

    #[test]
    fn test_path_length_ok() {
        let profile = windows_ntfs_profile();
        let result = check_path_length("/short/path/file.txt", &profile);
        assert!(result.path_ok);
        assert!(result.filename_ok);
        assert!(result.shortened.is_none());
    }

    #[test]
    fn test_filename_too_long() {
        let profile = ext4_profile();
        let long_name = "a".repeat(300);
        let result = check_path_length(&format!("/path/{long_name}"), &profile);
        assert!(!result.filename_ok);
        assert!(result.shortened.is_some());
        let shortened = result.shortened.unwrap();
        assert!(shortened.len() <= 255);
    }

    #[test]
    fn test_shorten_preserves_extension() {
        let mut rules = Vec::new();
        let long_name = format!("{}.txt", "a".repeat(300));
        let shortened = shorten_filename(&long_name, 255, &mut rules);
        assert!(shortened.len() <= 255);
        assert!(shortened.ends_with(".txt"));
        assert!(shortened.contains('~')); // hash suffix
    }

    #[test]
    fn test_onedrive_path_limit() {
        let profile = onedrive_profile();
        assert_eq!(profile.max_path_bytes, 400);
        let long_path = format!("/OneDrive/{}", "subdir/".repeat(60));
        let result = check_path_length(&long_path, &profile);
        assert!(!result.path_ok);
    }

    // ── CJK byte awareness (T-035) ──

    #[test]
    fn test_ext4_cjk_byte_length() {
        // Each CJK char is 3 bytes in UTF-8
        let cjk_name = "\u{6587}\u{4EF6}".repeat(43); // 86 chars = 258 bytes
        let (ok, byte_len) = check_ext4_byte_length(&cjk_name);
        assert!(!ok, "Should exceed 255 bytes (got {} bytes)", byte_len);
        assert!(byte_len > 255);
    }

    #[test]
    fn test_ext4_ascii_length() {
        let ascii_name = "a".repeat(255);
        let (ok, byte_len) = check_ext4_byte_length(&ascii_name);
        assert!(ok);
        assert_eq!(byte_len, 255);
    }

    #[test]
    fn test_ext4_mixed_script_length() {
        // Mix of ASCII (1 byte) and CJK (3 bytes)
        let name = format!("report_{}", "\u{6587}\u{4EF6}".repeat(42));
        let (ok, byte_len) = check_ext4_byte_length(&name);
        // 7 bytes + 252 bytes = 259 bytes
        assert!(!ok, "Expected over 255 bytes, got {byte_len}");
    }

    #[test]
    fn test_shorten_cjk_filename() {
        let mut rules = Vec::new();
        let cjk_name = format!("{}.txt", "\u{6587}\u{4EF6}".repeat(50)); // way over 255 bytes
        let shortened = shorten_filename(&cjk_name, 255, &mut rules);
        assert!(shortened.len() <= 255);
        assert!(shortened.ends_with(".txt"));
    }

    // ── Case collision detection (T-035) ──

    #[test]
    fn test_case_collision_detected() {
        let names = ["Report.txt", "report.txt", "REPORT.TXT"];
        let collisions = detect_case_collisions(&names);
        assert_eq!(collisions.len(), 1);
        assert_eq!(collisions[0].len(), 3);
    }

    #[test]
    fn test_no_case_collision() {
        let names = ["file1.txt", "file2.txt", "file3.doc"];
        let collisions = detect_case_collisions(&names);
        assert!(collisions.is_empty());
    }

    #[test]
    fn test_multiple_collision_groups() {
        let names = ["A.txt", "a.txt", "B.doc", "b.doc", "unique.pdf"];
        let collisions = detect_case_collisions(&names);
        assert_eq!(collisions.len(), 2);
    }

    // ── Auto-detection (T-036) ──

    #[test]
    fn test_auto_detect_s3() {
        assert_eq!(auto_detect_profile("s3://bucket/key"), "s3");
    }

    #[test]
    fn test_auto_detect_onedrive() {
        assert_eq!(auto_detect_profile("/Users/me/OneDrive/file.txt"), "onedrive");
    }

    #[test]
    fn test_auto_detect_dropbox() {
        assert_eq!(auto_detect_profile("/Users/me/Dropbox/file.txt"), "dropbox");
    }

    #[test]
    fn test_auto_detect_smb() {
        assert_eq!(auto_detect_profile("\\\\server\\share\\file.txt"), "smb-to-windows");
        assert_eq!(auto_detect_profile("smb://server/share"), "smb-to-windows");
    }

    #[test]
    fn test_auto_detect_b2() {
        assert_eq!(auto_detect_profile("b2://bucket/key"), "backblaze-b2");
    }

    #[test]
    fn test_profile_from_protocol() {
        assert_eq!(profile_from_protocol("s3"), "s3");
        assert_eq!(profile_from_protocol("onedrive"), "onedrive");
        assert_eq!(profile_from_protocol("google_drive"), "google-drive");
        assert_eq!(profile_from_protocol("dropbox"), "dropbox");
        assert_eq!(profile_from_protocol("smb"), "smb-to-windows");
    }

    // ── Profile-specific forbidden chars ──

    #[test]
    fn test_windows_ntfs_constraints() {
        let p = windows_ntfs_profile();
        assert!(p.forbidden_chars.contains(&'<'));
        assert!(p.no_trailing_dots);
        assert_eq!(p.case_sensitivity, CaseSensitivity::Insensitive);
    }

    #[test]
    fn test_ext4_constraints() {
        let p = ext4_profile();
        assert_eq!(p.forbidden_chars, vec!['/']);
        assert!(!p.no_trailing_dots);
        assert_eq!(p.case_sensitivity, CaseSensitivity::Sensitive);
        assert_eq!(p.max_filename_bytes, 255);
    }

    #[test]
    fn test_strict_enterprise_most_restrictive() {
        let p = strict_enterprise_profile();
        assert!(p.forbidden_chars.len() > 10);
        assert_eq!(p.max_filename_bytes, 100);
        assert_eq!(p.max_path_bytes, 200);
    }
}
