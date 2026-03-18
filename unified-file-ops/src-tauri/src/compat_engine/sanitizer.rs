//! Character sanitizer for cross-platform filename compatibility (T-034).
//!
//! Replaces Windows-forbidden characters with full-width Unicode equivalents
//! instead of underscores, making the transformation reversible.
//!
//! Mapping: ASCII -> Full-width Unicode (U+FF00 block)
//!   < -> ＜ (U+FF1C)    > -> ＞ (U+FF1E)    : -> ： (U+FF1A)
//!   " -> ＂ (U+FF02)    | -> ｜ (U+FF5C)    ? -> ？ (U+FF1F)
//!   * -> ＊ (U+FF0A)    / -> ／ (U+FF0F)    \ -> ＼ (U+FF3C)

use std::collections::HashMap;

/// A reversible character mapping entry.
#[derive(Debug, Clone)]
pub struct CharMapping {
    pub original: char,
    pub replacement: char,
    pub description: String,
}

/// Result of sanitization.
#[derive(Debug, Clone)]
pub struct SanitizeResult {
    pub original: String,
    pub sanitized: String,
    pub mappings: Vec<CharMapping>,
    pub rules_applied: Vec<String>,
    pub is_modified: bool,
    pub reversible: bool,
}

/// Full-width Unicode replacements for Windows-forbidden characters.
/// These are visually similar but distinct codepoints, making the mapping reversible.
const FULLWIDTH_MAP: &[(char, char, &str)] = &[
    ('<', '\u{FF1C}', "less-than to fullwidth"),
    ('>', '\u{FF1E}', "greater-than to fullwidth"),
    (':', '\u{FF1A}', "colon to fullwidth"),
    ('"', '\u{FF02}', "double-quote to fullwidth"),
    ('|', '\u{FF5C}', "pipe to fullwidth"),
    ('?', '\u{FF1F}', "question-mark to fullwidth"),
    ('*', '\u{FF0A}', "asterisk to fullwidth"),
    ('/', '\u{FF0F}', "forward-slash to fullwidth"),
    ('\\', '\u{FF3C}', "backslash to fullwidth"),
];

/// Build the forward (sanitize) mapping table.
fn forward_map() -> HashMap<char, (char, &'static str)> {
    FULLWIDTH_MAP
        .iter()
        .map(|&(orig, repl, desc)| (orig, (repl, desc)))
        .collect()
}

/// Build the reverse (restore) mapping table.
fn reverse_map() -> HashMap<char, char> {
    FULLWIDTH_MAP
        .iter()
        .map(|&(orig, repl, _)| (repl, orig))
        .collect()
}

/// Characters forbidden on Windows file systems.
pub const WINDOWS_FORBIDDEN: &[char] = &['<', '>', ':', '"', '|', '?', '*'];

/// Characters forbidden on macOS (in addition to NUL).
pub const MACOS_FORBIDDEN: &[char] = &[':', '/'];

/// Characters forbidden on Linux (in addition to NUL).
pub const LINUX_FORBIDDEN: &[char] = &['/'];

/// Sanitize a filename by replacing forbidden characters with full-width equivalents.
/// The `forbidden` set determines which characters to replace.
pub fn sanitize(name: &str, forbidden: &[char]) -> SanitizeResult {
    let fwd = forward_map();
    let mut result = String::with_capacity(name.len() * 2);
    let mut mappings = Vec::new();
    let mut rules = Vec::new();

    for ch in name.chars() {
        if forbidden.contains(&ch) {
            if let Some(&(replacement, desc)) = fwd.get(&ch) {
                result.push(replacement);
                mappings.push(CharMapping {
                    original: ch,
                    replacement,
                    description: desc.to_string(),
                });
                let rule = format!("Replaced '{}' (U+{:04X}) with fullwidth '{}' (U+{:04X})",
                    ch, ch as u32, replacement, replacement as u32);
                if !rules.contains(&rule) {
                    rules.push(rule);
                }
            } else {
                // No fullwidth equivalent; use underscore as fallback
                result.push('_');
                mappings.push(CharMapping {
                    original: ch,
                    replacement: '_',
                    description: format!("No fullwidth equivalent for U+{:04X}", ch as u32),
                });
                let rule = format!("Replaced '{}' (U+{:04X}) with '_'", ch, ch as u32);
                if !rules.contains(&rule) {
                    rules.push(rule);
                }
            }
        } else {
            result.push(ch);
        }
    }

    let is_modified = result != name;
    SanitizeResult {
        original: name.to_string(),
        sanitized: result,
        mappings,
        rules_applied: rules,
        is_modified,
        reversible: is_modified,
    }
}

/// Restore a sanitized filename back to its original by reversing full-width mappings.
pub fn restore(sanitized: &str) -> String {
    let rev = reverse_map();
    sanitized
        .chars()
        .map(|ch| *rev.get(&ch).unwrap_or(&ch))
        .collect()
}

/// Sanitize trailing dots and spaces (forbidden on Windows).
/// Returns the trimmed name and whether it was modified.
pub fn sanitize_trailing(name: &str) -> (String, Vec<String>) {
    let mut rules = Vec::new();
    let trimmed = name.trim_end_matches(['.', ' ']);

    if trimmed.len() != name.len() {
        let trailing = &name[trimmed.len()..];
        let dots = trailing.chars().filter(|&c| c == '.').count();
        let spaces = trailing.chars().filter(|&c| c == ' ').count();
        if dots > 0 {
            rules.push(format!("Removed {dots} trailing dot(s)"));
        }
        if spaces > 0 {
            rules.push(format!("Removed {spaces} trailing space(s)"));
        }
    }

    // Also check leading dots on Windows (not forbidden but hidden on Unix)
    // We don't modify leading dots as they are intentional on Unix.

    (trimmed.to_string(), rules)
}

/// Detect control characters (U+0000..U+001F, U+007F) which are forbidden
/// on most filesystems.
pub fn sanitize_control_chars(name: &str) -> (String, Vec<String>) {
    let mut rules = Vec::new();
    let mut result = String::with_capacity(name.len());
    let mut count = 0;

    for ch in name.chars() {
        if ch.is_control() {
            // Skip all control characters (including tab, which is invalid in filenames)
            count += 1;
        } else {
            result.push(ch);
        }
    }

    if count > 0 {
        rules.push(format!("Removed {count} control character(s)"));
    }

    (result, rules)
}

/// Get the forbidden characters for a given platform/profile.
pub fn forbidden_chars_for(profile: &str) -> Vec<char> {
    match profile {
        "windows" | "windows-ntfs" | "windows-fat32" | "onedrive" | "smb-to-windows"
        | "strict-enterprise" => {
            // Windows + NUL (NUL can't appear in Rust String)
            let mut chars = WINDOWS_FORBIDDEN.to_vec();
            // Also forbid forward/backslash in filenames (not paths)
            chars.push('/');
            chars.push('\\');
            chars
        }
        "macos" | "macos-apfs-ci" | "macos-apfs-cs" | "macos-hfs" => {
            MACOS_FORBIDDEN.to_vec()
        }
        "linux" | "ext4" => {
            LINUX_FORBIDDEN.to_vec()
        }
        "google-drive" | "dropbox" => {
            // Cloud services inherit Windows restrictions for compatibility
            let mut chars = WINDOWS_FORBIDDEN.to_vec();
            chars.push('/');
            chars.push('\\');
            chars
        }
        "s3" | "backblaze-b2" => {
            // S3 and B2 have very few restrictions but we still sanitize
            // control characters and some edge cases
            vec![]
        }
        _ => {
            // Universal/cross-platform: apply all restrictions
            let mut chars = WINDOWS_FORBIDDEN.to_vec();
            chars.extend_from_slice(MACOS_FORBIDDEN);
            chars.push('\\');
            chars.sort();
            chars.dedup();
            chars
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Fullwidth replacement ──

    #[test]
    fn test_sanitize_windows_forbidden() {
        let result = sanitize("file<name>.txt", WINDOWS_FORBIDDEN);
        assert!(result.is_modified);
        assert!(result.reversible);
        assert!(!result.sanitized.contains('<'));
        assert!(!result.sanitized.contains('>'));
        assert!(result.sanitized.contains('\u{FF1C}'));
        assert!(result.sanitized.contains('\u{FF1E}'));
    }

    #[test]
    fn test_sanitize_colon() {
        let result = sanitize("file:name.txt", &[':']);
        assert_eq!(result.sanitized, "file\u{FF1A}name.txt");
    }

    #[test]
    fn test_sanitize_all_forbidden() {
        let name = "a<b>c:d\"e|f?g*h";
        let result = sanitize(name, WINDOWS_FORBIDDEN);
        assert!(result.is_modified);
        assert_eq!(result.mappings.len(), 7);
    }

    #[test]
    fn test_sanitize_no_changes() {
        let result = sanitize("normal_file.txt", WINDOWS_FORBIDDEN);
        assert!(!result.is_modified);
        assert!(result.mappings.is_empty());
    }

    // ── Reversibility ──

    #[test]
    fn test_restore_fullwidth() {
        let original = "file<name>:test.txt";
        let sanitized = sanitize(original, WINDOWS_FORBIDDEN);
        let restored = restore(&sanitized.sanitized);
        assert_eq!(restored, original);
    }

    #[test]
    fn test_restore_all_chars() {
        let original = "a<b>c:d\"e|f?g*h/i\\j";
        let forbidden = &['<', '>', ':', '"', '|', '?', '*', '/', '\\'];
        let sanitized = sanitize(original, forbidden);
        let restored = restore(&sanitized.sanitized);
        assert_eq!(restored, original);
    }

    #[test]
    fn test_restore_unchanged() {
        let result = restore("normal_file.txt");
        assert_eq!(result, "normal_file.txt");
    }

    // ── Trailing dots/spaces ──

    #[test]
    fn test_sanitize_trailing_dots() {
        let (result, rules) = sanitize_trailing("file...");
        assert_eq!(result, "file");
        assert!(!rules.is_empty());
    }

    #[test]
    fn test_sanitize_trailing_spaces() {
        let (result, rules) = sanitize_trailing("file   ");
        assert_eq!(result, "file");
        assert!(!rules.is_empty());
    }

    #[test]
    fn test_sanitize_trailing_mixed() {
        let (result, rules) = sanitize_trailing("file. . ");
        assert_eq!(result, "file");
        assert_eq!(rules.len(), 2); // dots + spaces
    }

    #[test]
    fn test_no_trailing_issues() {
        let (result, rules) = sanitize_trailing("file.txt");
        assert_eq!(result, "file.txt");
        assert!(rules.is_empty());
    }

    // ── Control characters ──

    #[test]
    fn test_sanitize_control_chars() {
        let name = "file\x01name\x02.txt";
        let (result, rules) = sanitize_control_chars(name);
        assert_eq!(result, "filename.txt");
        assert!(!rules.is_empty());
    }

    #[test]
    fn test_sanitize_null_byte() {
        // NUL bytes can't appear in Rust &str, but control chars can
        let name = "file\x7Fname.txt"; // DEL character
        let (result, _rules) = sanitize_control_chars(name);
        assert_eq!(result, "filename.txt");
    }

    // ── Platform-specific forbidden chars ──

    #[test]
    fn test_forbidden_chars_windows() {
        let chars = forbidden_chars_for("windows");
        assert!(chars.contains(&'<'));
        assert!(chars.contains(&'>'));
        assert!(chars.contains(&':'));
        assert!(chars.contains(&'/'));
        assert!(chars.contains(&'\\'));
    }

    #[test]
    fn test_forbidden_chars_macos() {
        let chars = forbidden_chars_for("macos");
        assert!(chars.contains(&':'));
        assert!(chars.contains(&'/'));
        assert!(!chars.contains(&'<'));
    }

    #[test]
    fn test_forbidden_chars_linux() {
        let chars = forbidden_chars_for("linux");
        assert!(chars.contains(&'/'));
        assert!(!chars.contains(&':'));
    }

    #[test]
    fn test_forbidden_chars_cloud() {
        let chars = forbidden_chars_for("onedrive");
        assert!(chars.contains(&'<'));
        assert!(chars.contains(&'/'));
    }

    // ── Integration: sanitize + restore cycle ──

    #[test]
    fn test_full_cycle_unicode_filename() {
        let name = "R\u{00E9}sum\u{00E9}: The <Best> Version?.pdf";
        let forbidden = forbidden_chars_for("windows");
        let sanitized = sanitize(name, &forbidden);
        assert!(sanitized.is_modified);
        let restored = restore(&sanitized.sanitized);
        assert_eq!(restored, name);
    }

    #[test]
    fn test_full_cycle_cjk() {
        let name = "\u{6587}\u{4EF6}<\u{540D}>.txt";
        let sanitized = sanitize(name, WINDOWS_FORBIDDEN);
        let restored = restore(&sanitized.sanitized);
        assert_eq!(restored, name);
    }

    #[test]
    fn test_full_cycle_emoji() {
        let name = "\u{1F600}file?name*.txt";
        let sanitized = sanitize(name, WINDOWS_FORBIDDEN);
        let restored = restore(&sanitized.sanitized);
        assert_eq!(restored, name);
    }
}
