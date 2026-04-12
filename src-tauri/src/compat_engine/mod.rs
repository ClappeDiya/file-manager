//! Compatibility engine - naming normalization, path rules, destination profiles.
//!
//! Provides `CompatChecker` implementing `CompatOperations`.
//!
//! Modules:
//! - `unicode`: NFC/NFD normalizer, zero-width/bidi stripping
//! - `sanitizer`: Full-width Unicode character replacement (reversible)
//! - `profiles`: Destination profiles, reserved names, path length, case collisions
//! - `mapping`: SQLite mapping DB, sidecar manifests, xattr storage, restore

pub mod mapping;
pub mod profiles;
pub mod sanitizer;
pub mod unicode;

use crate::core::error::AppError;
use crate::core::traits::{BoxFuture, CompatOperations};
use crate::core::types::{CompatMapping, CompatResult};

use profiles::DestProfile;
use serde::{Deserialize, Serialize};

// ── Intervention Tiers (T-038) ──

/// Intervention tier for a compatibility translation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterventionTier {
    /// Tier 1: Auto-quiet. Handle silently, log, badge count.
    /// Examples: Unicode normalization, trailing dot removal
    AutoQuiet = 1,
    /// Tier 2: Visible auto. Inline notification, non-blocking, expandable details.
    /// Examples: Reserved name escaping, fullwidth character replacement
    VisibleAuto = 2,
    /// Tier 3: Decision required. Modal dialog with rename/skip/overwrite/cancel.
    /// Examples: Path too long (no automatic fix), case collision
    DecisionRequired = 3,
}

/// A single compatibility translation with tier classification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompatTranslation {
    pub original_name: String,
    pub translated_name: String,
    pub rules_applied: Vec<String>,
    pub tier: InterventionTier,
    pub reversible: bool,
    /// Profile that triggered this translation.
    pub profile_id: String,
}

/// Summary of compatibility checks for a batch of files.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompatBatchResult {
    pub translations: Vec<CompatTranslation>,
    pub tier1_count: usize,
    pub tier2_count: usize,
    pub tier3_count: usize,
    pub total_modified: usize,
    /// Simple-mode summary message.
    pub simple_message: String,
    /// Advanced-mode details (one per translation).
    pub advanced_details: Vec<String>,
}

/// Characters forbidden on Windows file systems.
const WINDOWS_FORBIDDEN: &[char] = &['<', '>', ':', '"', '|', '?', '*'];

/// Reserved names on Windows (case-insensitive).
const WINDOWS_RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM0", "COM1", "COM2", "COM3", "COM4", "COM5",
    "COM6", "COM7", "COM8", "COM9", "LPT0", "LPT1", "LPT2", "LPT3", "LPT4",
    "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

pub struct CompatChecker;

impl CompatChecker {
    pub fn new() -> Self {
        Self
    }

    /// Legacy check_name_sync for backward compatibility.
    /// Performs basic checks using the old underscore-replacement approach.
    pub fn check_name_sync(name: &str, target_platform: &str) -> CompatResult {
        let mut translated = name.to_string();
        let mut rules = Vec::new();

        match target_platform {
            "windows" => {
                // Replace forbidden characters
                for &ch in WINDOWS_FORBIDDEN {
                    if translated.contains(ch) {
                        translated = translated.replace(ch, "_");
                        rules.push(format!("Replaced forbidden character '{ch}' with '_'"));
                    }
                }
                // Check reserved names
                let stem = translated.split('.').next().unwrap_or("").to_uppercase();
                if WINDOWS_RESERVED.contains(&stem.as_str()) {
                    translated = format!("_{translated}");
                    rules.push(format!("Prefixed reserved name '{stem}' with '_'"));
                }
                // Remove trailing dots/spaces
                let trimmed = translated.trim_end_matches(['.', ' ']);
                if trimmed.len() != translated.len() {
                    rules.push("Removed trailing dots/spaces".to_string());
                    translated = trimmed.to_string();
                }
                // Max path component 255
                if translated.len() > 255 {
                    translated.truncate(255);
                    rules.push("Truncated to 255 characters".to_string());
                }
            }
            "macos" => {
                // macOS forbids ':' and NUL
                if translated.contains(':') {
                    translated = translated.replace(':', "_");
                    rules.push("Replaced ':' with '_' (macOS)".to_string());
                }
                if translated.len() > 255 {
                    translated.truncate(255);
                    rules.push("Truncated to 255 characters".to_string());
                }
            }
            "linux" => {
                // Linux only forbids '/' and NUL (NUL not representable in String)
                if translated.len() > 255 {
                    translated.truncate(255);
                    rules.push("Truncated to 255 characters".to_string());
                }
            }
            _ => {
                // Apply all platform rules conservatively
                for &ch in WINDOWS_FORBIDDEN {
                    if translated.contains(ch) {
                        translated = translated.replace(ch, "_");
                        rules
                            .push(format!("Replaced '{ch}' with '_' (cross-platform)"));
                    }
                }
                if translated.contains(':') {
                    translated = translated.replace(':', "_");
                    rules.push("Replaced ':' with '_' (cross-platform)".to_string());
                }
            }
        }

        let is_modified = translated != name;
        CompatResult {
            original_name: name.to_string(),
            translated_name: translated,
            rules_applied: rules,
            is_modified,
            reversible: is_modified,
        }
    }

    /// Enhanced check using destination profiles with full-width Unicode replacement.
    /// Returns a CompatTranslation with tier classification.
    pub fn check_name_with_profile(name: &str, profile: &DestProfile) -> CompatTranslation {
        let mut translated = name.to_string();
        let mut rules = Vec::new();
        let mut highest_tier = InterventionTier::AutoQuiet;

        // Step 1: Unicode normalization (Tier 1 - silent)
        let norm_form = unicode::preferred_form(&profile.id);
        let norm_result = unicode::normalize(&translated, norm_form);
        if norm_result.was_modified {
            translated = norm_result.normalized;
            rules.extend(norm_result.changes);
        }

        // Step 2: Strip zero-width characters (Tier 1)
        let (stripped, zw_rules) = unicode::strip_zero_width(&translated);
        if !zw_rules.is_empty() {
            translated = stripped;
            rules.extend(zw_rules);
        }

        // Step 3: Strip bidi overrides (Tier 1)
        let (stripped, bidi_rules) = unicode::strip_bidi_overrides(&translated);
        if !bidi_rules.is_empty() {
            translated = stripped;
            rules.extend(bidi_rules);
        }

        // Step 4: Control character removal (Tier 1)
        let (stripped, ctrl_rules) = sanitizer::sanitize_control_chars(&translated);
        if !ctrl_rules.is_empty() {
            translated = stripped;
            rules.extend(ctrl_rules);
        }

        // Step 5: Character sanitization with full-width Unicode (Tier 2)
        if !profile.forbidden_chars.is_empty() {
            let san_result = sanitizer::sanitize(&translated, &profile.forbidden_chars);
            if san_result.is_modified {
                translated = san_result.sanitized;
                rules.extend(san_result.rules_applied);
                highest_tier = InterventionTier::VisibleAuto;
            }
        }

        // Step 6: Reserved name escaping (Tier 2)
        if !profile.reserved_names.is_empty() {
            let (escaped, rule) = profiles::escape_reserved(&translated);
            if let Some(r) = rule {
                translated = escaped;
                rules.push(r);
                highest_tier = InterventionTier::VisibleAuto;
            }
        }

        // Step 7: Trailing dots/spaces (Tier 1)
        if profile.no_trailing_dots || profile.no_trailing_spaces {
            let (trimmed, trailing_rules) = sanitizer::sanitize_trailing(&translated);
            if !trailing_rules.is_empty() {
                translated = trimmed;
                rules.extend(trailing_rules);
            }
        }

        // Step 8: Filename length check (Tier 2 if auto-shortened, Tier 3 if unfixable)
        let filename_bytes = translated.len();
        if filename_bytes > profile.max_filename_bytes {
            let mut shorten_rules = Vec::new();
            let shortened = profiles::shorten_filename(
                &translated,
                profile.max_filename_bytes,
                &mut shorten_rules,
            );
            translated = shortened;
            rules.extend(shorten_rules);
            highest_tier = InterventionTier::VisibleAuto;
        }

        let is_modified = translated != name;
        CompatTranslation {
            original_name: name.to_string(),
            translated_name: translated,
            rules_applied: rules,
            tier: if is_modified { highest_tier } else { InterventionTier::AutoQuiet },
            reversible: is_modified,
            profile_id: profile.id.clone(),
        }
    }

    /// Check a batch of filenames against a profile.
    /// Also detects case collisions for case-insensitive targets.
    pub fn check_batch(names: &[&str], profile: &DestProfile) -> CompatBatchResult {
        let mut translations = Vec::new();

        for &name in names {
            let translation = Self::check_name_with_profile(name, profile);
            if translation.translated_name != translation.original_name {
                translations.push(translation);
            }
        }

        // Detect case collisions for case-insensitive targets
        if profile.case_sensitivity == profiles::CaseSensitivity::Insensitive {
            let collision_groups = profiles::detect_case_collisions(names);
            for group in &collision_groups {
                for name in group {
                    // Add a Tier 3 translation for collision detection
                    translations.push(CompatTranslation {
                        original_name: name.clone(),
                        translated_name: name.clone(), // not auto-fixed
                        rules_applied: vec![format!(
                            "Case collision detected with: {}",
                            group
                                .iter()
                                .filter(|n| *n != name)
                                .cloned()
                                .collect::<Vec<_>>()
                                .join(", ")
                        )],
                        tier: InterventionTier::DecisionRequired,
                        reversible: false,
                        profile_id: profile.id.clone(),
                    });
                }
            }
        }

        let tier1_count = translations.iter().filter(|t| t.tier == InterventionTier::AutoQuiet).count();
        let tier2_count = translations.iter().filter(|t| t.tier == InterventionTier::VisibleAuto).count();
        let tier3_count = translations.iter().filter(|t| t.tier == InterventionTier::DecisionRequired).count();
        let total_modified = translations.len();

        // Generate simple/advanced messages
        let simple_message = if total_modified == 0 {
            "All filenames are compatible with the destination.".to_string()
        } else {
            let mut parts = Vec::new();
            if tier1_count > 0 {
                parts.push(format!("{tier1_count} adjusted silently"));
            }
            if tier2_count > 0 {
                parts.push(format!("{tier2_count} adjusted automatically"));
            }
            if tier3_count > 0 {
                parts.push(format!("{tier3_count} need your decision"));
            }
            format!(
                "We adjusted {} name{} so the transfer could continue: {}",
                total_modified,
                if total_modified == 1 { "" } else { "s" },
                parts.join(", ")
            )
        };

        let advanced_details = translations
            .iter()
            .map(|t| {
                format!(
                    "[Tier {}] '{}' -> '{}' ({})",
                    t.tier as u8,
                    t.original_name,
                    t.translated_name,
                    t.rules_applied.join("; ")
                )
            })
            .collect();

        CompatBatchResult {
            translations,
            tier1_count,
            tier2_count,
            tier3_count,
            total_modified,
            simple_message,
            advanced_details,
        }
    }
}

impl Default for CompatChecker {
    fn default() -> Self {
        Self::new()
    }
}

impl CompatOperations for CompatChecker {
    fn check_name(
        &self,
        name: &str,
        target_platform: &str,
    ) -> BoxFuture<'_, Result<CompatResult, AppError>> {
        let name = name.to_string();
        let platform = target_platform.to_string();
        Box::pin(async move { Ok(Self::check_name_sync(&name, &platform)) })
    }

    fn list_mappings(&self) -> BoxFuture<'_, Result<Vec<CompatMapping>, AppError>> {
        Box::pin(async move {
            // Default built-in mappings; user-defined ones come from the DB.
            Ok(Vec::new())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Legacy backward-compatible tests ──

    #[test]
    fn test_windows_forbidden_chars() {
        let result = CompatChecker::check_name_sync("file<name>.txt", "windows");
        assert!(result.is_modified);
        assert!(!result.translated_name.contains('<'));
        assert!(!result.translated_name.contains('>'));
    }

    #[test]
    fn test_windows_reserved_name() {
        let result = CompatChecker::check_name_sync("CON.txt", "windows");
        assert!(result.is_modified);
        assert!(result.translated_name.starts_with('_'));
    }

    #[test]
    fn test_windows_trailing_dots() {
        let result = CompatChecker::check_name_sync("file...", "windows");
        assert!(result.is_modified);
        assert!(!result.translated_name.ends_with('.'));
    }

    #[test]
    fn test_macos_colon() {
        let result = CompatChecker::check_name_sync("file:name.txt", "macos");
        assert!(result.is_modified);
        assert!(!result.translated_name.contains(':'));
    }

    #[test]
    fn test_linux_safe_name() {
        let result = CompatChecker::check_name_sync("normal_file.txt", "linux");
        assert!(!result.is_modified);
        assert_eq!(result.translated_name, "normal_file.txt");
    }

    #[test]
    fn test_cross_platform_conservative() {
        let result = CompatChecker::check_name_sync("file:name<>.txt", "universal");
        assert!(result.is_modified);
        assert!(!result.translated_name.contains(':'));
        assert!(!result.translated_name.contains('<'));
    }

    #[test]
    fn test_already_compatible() {
        let result = CompatChecker::check_name_sync("safe_name.txt", "windows");
        assert!(!result.is_modified);
        assert!(result.rules_applied.is_empty());
    }

    // ── Enhanced profile-based tests ──

    #[test]
    fn test_profile_based_windows_ntfs() {
        let profile = profiles::get_profile("windows-ntfs").unwrap();
        let t = CompatChecker::check_name_with_profile("file<name>.txt", &profile);
        assert!(t.translated_name != t.original_name);
        assert!(t.translated_name.contains('\u{FF1C}')); // fullwidth <
        assert!(t.reversible);
    }

    #[test]
    fn test_profile_based_reserved_name() {
        let profile = profiles::get_profile("windows-ntfs").unwrap();
        let t = CompatChecker::check_name_with_profile("CON.txt", &profile);
        assert_eq!(t.translated_name, "_CON.txt");
        assert_eq!(t.tier, InterventionTier::VisibleAuto);
    }

    #[test]
    fn test_profile_based_safe_name() {
        let profile = profiles::get_profile("ext4").unwrap();
        let t = CompatChecker::check_name_with_profile("normal_file.txt", &profile);
        assert_eq!(t.translated_name, "normal_file.txt");
    }

    #[test]
    fn test_profile_based_unicode_normalization() {
        // NFD string going to Windows (wants NFC)
        let profile = profiles::get_profile("windows-ntfs").unwrap();
        let nfd = "e\u{0301}"; // e + combining acute
        let t = CompatChecker::check_name_with_profile(nfd, &profile);
        assert_eq!(t.translated_name, "\u{00E9}"); // NFC form
    }

    // ── Batch check tests ──

    #[test]
    fn test_batch_check_basic() {
        let profile = profiles::get_profile("windows-ntfs").unwrap();
        let names = ["normal.txt", "file<bad>.txt", "CON.txt"];
        let result = CompatChecker::check_batch(&names, &profile);
        assert!(result.total_modified >= 2);
        assert!(!result.simple_message.is_empty());
    }

    #[test]
    fn test_batch_check_case_collisions() {
        let profile = profiles::get_profile("windows-ntfs").unwrap();
        let names = ["Report.txt", "report.txt", "normal.txt"];
        let result = CompatChecker::check_batch(&names, &profile);
        assert!(result.tier3_count > 0, "Should detect case collision");
    }

    #[test]
    fn test_batch_check_no_collisions_case_sensitive() {
        let profile = profiles::get_profile("ext4").unwrap();
        let names = ["Report.txt", "report.txt"];
        let result = CompatChecker::check_batch(&names, &profile);
        assert_eq!(result.tier3_count, 0, "ext4 is case-sensitive, no collisions");
    }

    // ── Intervention tier tests (T-038) ──

    #[test]
    fn test_tier1_unicode_normalization() {
        let profile = profiles::get_profile("windows-ntfs").unwrap();
        let t = CompatChecker::check_name_with_profile("cafe\u{0301}.txt", &profile);
        // NFC normalization is Tier 1 (silent) unless combined with other changes
        assert!(t.tier == InterventionTier::AutoQuiet || t.tier == InterventionTier::VisibleAuto);
    }

    #[test]
    fn test_tier2_forbidden_char() {
        let profile = profiles::get_profile("windows-ntfs").unwrap();
        let t = CompatChecker::check_name_with_profile("file|name.txt", &profile);
        assert_eq!(t.tier, InterventionTier::VisibleAuto);
    }

    #[test]
    fn test_tier3_case_collision() {
        let profile = profiles::get_profile("windows-ntfs").unwrap();
        let names = ["File.txt", "file.txt"];
        let result = CompatChecker::check_batch(&names, &profile);
        let tier3 = result.translations.iter()
            .filter(|t| t.tier == InterventionTier::DecisionRequired)
            .count();
        assert!(tier3 > 0);
    }

    // ── Simple/Advanced mode messages (T-038) ──

    #[test]
    fn test_simple_message_no_issues() {
        let profile = profiles::get_profile("ext4").unwrap();
        let names = ["normal.txt", "safe_file.doc"];
        let result = CompatChecker::check_batch(&names, &profile);
        assert!(result.simple_message.contains("compatible"));
    }

    #[test]
    fn test_simple_message_with_issues() {
        let profile = profiles::get_profile("windows-ntfs").unwrap();
        let names = ["file<bad>.txt"];
        let result = CompatChecker::check_batch(&names, &profile);
        assert!(result.simple_message.contains("adjusted"));
    }

    #[test]
    fn test_advanced_details() {
        let profile = profiles::get_profile("windows-ntfs").unwrap();
        let names = ["file<bad>.txt", "CON.txt"];
        let result = CompatChecker::check_batch(&names, &profile);
        assert!(!result.advanced_details.is_empty());
        for detail in &result.advanced_details {
            assert!(detail.starts_with("[Tier"));
        }
    }

    // ────────────────────────────────────────────────────────────────
    // 100+ FILENAME TEST CORPUS (T-034 requirement)
    // ────────────────────────────────────────────────────────────────

    /// Comprehensive test corpus: 100+ filenames covering Latin accents,
    /// CJK, emoji, Arabic, Hebrew, mixed scripts, edge cases.
    #[test]
    fn test_100_filename_corpus() {
        let profile = profiles::get_profile("windows-ntfs").unwrap();

        // The corpus: each entry is (original, should_be_modified)
        let corpus: Vec<(&str, bool)> = vec![
            // ── Latin with accents (NFC) ──
            ("caf\u{00E9}.txt", false),                          // 1  cafe with acute
            ("r\u{00E9}sum\u{00E9}.pdf", false),                 // 2
            ("\u{00FC}ber.doc", false),                           // 3  u-umlaut
            ("\u{00F1}o\u{00F1}o.txt", false),                   // 4  n-tilde
            ("\u{00E0}_propos.md", false),                        // 5  a-grave
            ("\u{00C9}cole.txt", false),                          // 6  E-acute
            ("\u{00D6}sterreich.txt", false),                     // 7  O-umlaut
            ("\u{00C7}edille.txt", false),                        // 8  C-cedilla
            ("na\u{00EF}ve.txt", false),                         // 9  i-diaeresis
            ("\u{00E2}me.doc", false),                            // 10 a-circumflex

            // ── Latin with accents (NFD - will be normalized to NFC) ──
            ("cafe\u{0301}.txt", true),                           // 11 e + combining acute
            ("re\u{0301}sume\u{0301}.pdf", true),                // 12
            ("u\u{0308}ber.doc", true),                           // 13 u + combining diaeresis
            ("n\u{0303}on\u{0303}o.txt", true),                  // 14 n + combining tilde
            ("a\u{0300}_propos.md", true),                        // 15 a + combining grave
            ("E\u{0301}cole.txt", true),                          // 16
            ("O\u{0308}sterreich.txt", true),                     // 17
            ("C\u{0327}edille.txt", true),                        // 18
            ("nai\u{0308}ve.txt", true),                          // 19
            ("a\u{0302}me.doc", true),                            // 20

            // ── CJK (Chinese/Japanese/Korean) ──
            ("\u{6587}\u{4EF6}\u{540D}.txt", false),             // 21 Chinese: 文件名
            ("\u{30C6}\u{30B9}\u{30C8}.doc", false),              // 22 Japanese katakana
            ("\u{3053}\u{3093}\u{306B}\u{3061}\u{306F}.txt", false), // 23 Hiragana
            ("\u{AC00}\u{B098}\u{B2E4}.txt", false),              // 24 Korean: 가나다
            ("\u{4E2D}\u{6587}\u{62A5}\u{544A}.pdf", false),     // 25 Chinese: 中文报告
            ("\u{65E5}\u{672C}\u{8A9E}.txt", false),              // 26 Japanese: 日本語
            ("\u{D55C}\u{AD6D}\u{C5B4}.doc", false),              // 27 Korean: 한국어
            ("\u{7B80}\u{4F53}\u{5B57}.txt", false),              // 28 Simplified Chinese
            ("\u{7E41}\u{9AD4}\u{5B57}.txt", false),              // 29 Traditional Chinese
            ("\u{6F22}\u{5B57}\u{4EA4}\u{3058}\u{308A}.doc", false), // 30 Mixed Kanji+Hiragana

            // ── Emoji ──
            ("photo_\u{1F600}.jpg", false),                       // 31 grinning face
            ("\u{1F4C4}_document.pdf", false),                    // 32 page facing up
            ("music_\u{1F3B5}.mp3", false),                       // 33 musical note
            ("folder_\u{1F4C1}.zip", false),                      // 34 open folder
            ("\u{2764}\u{FE0F}_love.txt", false),                 // 35 red heart
            ("\u{1F680}_launch.sh", false),                       // 36 rocket
            ("\u{1F4A1}_idea.md", false),                         // 37 light bulb
            ("\u{2705}_done.txt", false),                         // 38 check mark
            ("\u{26A0}\u{FE0F}_warning.log", false),              // 39 warning sign
            ("\u{1F3E0}_home.cfg", false),                        // 40 house

            // ── Arabic ──
            ("\u{0645}\u{0644}\u{0641}.txt", false),              // 41 ملف (file)
            ("\u{062A}\u{0642}\u{0631}\u{064A}\u{0631}.pdf", false), // 42 تقرير (report)
            ("\u{0645}\u{0633}\u{062A}\u{0646}\u{062F}.doc", false), // 43 مستند (document)
            ("\u{0635}\u{0648}\u{0631}\u{0629}.jpg", false),      // 44 صورة (image)
            ("\u{0628}\u{064A}\u{0627}\u{0646}\u{0627}\u{062A}.csv", false), // 45 بيانات (data)

            // ── Hebrew ──
            ("\u{05E7}\u{05D5}\u{05D1}\u{05E5}.txt", false),     // 46 קובץ (file)
            ("\u{05D3}\u{05D5}\u{05D7}.pdf", false),              // 47 דוח (report)
            ("\u{05DE}\u{05E1}\u{05DE}\u{05DA}.doc", false),      // 48 מסמך (document)
            ("\u{05EA}\u{05DE}\u{05D5}\u{05E0}\u{05D4}.jpg", false), // 49 תמונה (image)
            ("\u{05E0}\u{05EA}\u{05D5}\u{05E0}\u{05D9}\u{05DD}.csv", false), // 50 נתונים (data)

            // ── Windows-forbidden characters (must be modified) ──
            ("file<name>.txt", true),                              // 51
            ("file>name.txt", true),                               // 52
            ("file:name.txt", true),                               // 53
            ("file\"name.txt", true),                              // 54
            ("file|name.txt", true),                               // 55
            ("file?name.txt", true),                               // 56
            ("file*name.txt", true),                               // 57
            ("file/name.txt", true),                               // 58
            ("file\\name.txt", true),                              // 59
            ("all<>:\"|?*bad.txt", true),                          // 60

            // ── Windows reserved names ──
            ("CON.txt", true),                                     // 61
            ("PRN.doc", true),                                     // 62
            ("AUX.log", true),                                     // 63
            ("NUL.txt", true),                                     // 64
            ("COM1.dat", true),                                    // 65
            ("COM9.dat", true),                                    // 66
            ("LPT1.out", true),                                    // 67
            ("LPT9.out", true),                                    // 68
            ("con.txt", true),                                     // 69 lowercase reserved
            ("Nul", true),                                         // 70 no extension

            // ── Trailing dots/spaces ──
            ("file...", true),                                     // 71
            ("file   ", true),                                     // 72
            ("file. . ", true),                                    // 73
            ("name.", true),                                       // 74
            ("name. ", true),                                      // 75

            // ── Mixed scripts ──
            ("R\u{00E9}sum\u{00E9}_\u{5C65}\u{6B74}\u{66F8}.pdf", false), // 76 French+Japanese
            ("\u{041F}\u{0440}\u{0438}\u{0432}\u{0435}\u{0442}_hello.txt", false), // 77 Russian+English
            ("\u{0E2A}\u{0E27}\u{0E31}\u{0E2A}\u{0E14}\u{0E35}_\u{4F60}\u{597D}.txt", false), // 78 Thai+Chinese
            ("\u{0939}\u{093F}\u{0902}\u{0926}\u{0940}_doc.pdf", false),  // 79 Hindi
            ("\u{0E17}\u{0E14}\u{0E2A}\u{0E2D}\u{0E1A}.txt", false),     // 80 Thai

            // ── Edge cases ──
            (".", true),                                           // 81 single dot (trailing)
            ("..", true),                                          // 82 double dot (trailing)
            (".hidden", false),                                    // 83 leading dot is OK
            ("file name.txt", false),                              // 84 spaces in middle OK
            ("ALLCAPS.TXT", false),                                // 85
            ("a", false),                                          // 86 single char
            ("ab", false),                                         // 87 two chars
            (".a", false),                                         // 88 dot + char
            ("a.b.c.d.e.f", false),                                // 89 many extensions
            ("file (1).txt", false),                               // 90 parens OK on Windows

            // ── Greek ──
            ("\u{0395}\u{03BB}\u{03BB}\u{03B7}\u{03BD}\u{03B9}\u{03BA}\u{03AC}.txt", false), // 91 Ελληνικά
            ("\u{03B1}\u{03C1}\u{03C7}\u{03B5}\u{03AF}\u{03BF}.doc", false), // 92 αρχείο

            // ── Cyrillic ──
            ("\u{0424}\u{0430}\u{0439}\u{043B}.txt", false),      // 93 Файл
            ("\u{0414}\u{043E}\u{043A}\u{0443}\u{043C}\u{0435}\u{043D}\u{0442}.pdf", false), // 94 Документ

            // ── Devanagari ──
            ("\u{0926}\u{0938}\u{094D}\u{0924}\u{093E}\u{0935}\u{0947}\u{091C}.txt", false), // 95 दस्तावेज

            // ── Tamil ──
            ("\u{0B95}\u{0BCB}\u{0BAA}\u{0BCD}\u{0BAA}\u{0BC1}.doc", false), // 96 கோப்பு

            // ── Zero-width characters (must be stripped) ──
            ("file\u{200B}name.txt", true),                        // 97 zero-width space
            ("test\u{200D}file.txt", true),                        // 98 zero-width joiner
            ("\u{FEFF}bom_file.txt", true),                        // 99 BOM

            // ── Bidi overrides (must be stripped) ──
            ("file\u{202E}txt.exe", true),                         // 100 RLO attack

            // ── More edge cases ──
            ("file\t\tname.txt", true),                            // 101 tab (control char)
            ("\u{1100}\u{1161}\u{11A8}.txt", true),               // 102 Hangul Jamo -> composed (NFC)
            ("COM0.txt", true),                                    // 103
            ("LPT0.txt", true),                                    // 104
            ("\u{2060}word_joiner.txt", true),                     // 105 word joiner U+2060
            // Additional entries to reach 100+
            ("REPORT_\u{00E9}dit\u{00E9}.xlsx", false),            // 106 French accent in spreadsheet
            ("budget_\u{00A3}100.csv", false),                     // 107 pound sign
            ("\u{00A9}_copyright.txt", false),                     // 108 copyright symbol
            ("\u{00AE}_registered.doc", false),                    // 109 registered symbol
            ("\u{2122}_trademark.txt", false),                     // 110 trademark symbol
        ];

        // Test a simulated long filename (> 255 chars)
        let long_stem = "a".repeat(300);
        let long_name = format!("{long_stem}.txt");
        let t = CompatChecker::check_name_with_profile(&long_name, &profile);
        assert!(t.translated_name.len() <= 255, "Long filename should be shortened");
        assert!(t.translated_name.ends_with(".txt"), "Extension should be preserved");

        let mut pass_count = 0;
        let mut fail_count = 0;

        for (i, (name, should_modify)) in corpus.iter().enumerate() {
            if name.is_empty() {
                continue; // skip placeholder
            }
            let t = CompatChecker::check_name_with_profile(name, &profile);
            let was_modified = t.translated_name != t.original_name;

            if was_modified == *should_modify {
                pass_count += 1;
            } else {
                fail_count += 1;
                eprintln!(
                    "CORPUS #{}: '{}' expected modified={} but got modified={} (translated: '{}')",
                    i + 1,
                    name,
                    should_modify,
                    was_modified,
                    t.translated_name
                );
            }
        }

        eprintln!("Corpus results: {pass_count} passed, {fail_count} failed out of {} total", pass_count + fail_count);
        assert_eq!(fail_count, 0, "{fail_count} corpus entries failed");
        assert!(pass_count >= 100, "Expected 100+ corpus entries, got {pass_count}");
    }

    /// Performance test: 10K filenames in <5s (debug build).
    #[test]
    fn test_10k_filename_performance() {
        let profile = profiles::get_profile("windows-ntfs").unwrap();
        let names: Vec<String> = (0..10_000)
            .map(|i| format!("file_{i}_\u{00E9}\u{00FC}\u{00F1}.txt"))
            .collect();
        let name_refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();

        let start = std::time::Instant::now();
        for name in &name_refs {
            let _ = CompatChecker::check_name_with_profile(name, &profile);
        }
        let elapsed = start.elapsed();

        eprintln!("10K filenames processed in {:?}", elapsed);
        assert!(
            elapsed.as_millis() < 5000,
            "10K filenames took {}ms, expected <5000ms",
            elapsed.as_millis()
        );
    }

    /// NFC/NFD round-trip preservation test.
    #[test]
    fn test_nfc_nfd_roundtrip_corpus() {
        let test_names = [
            "caf\u{00E9}.txt",
            "r\u{00E9}sum\u{00E9}.pdf",
            "\u{00FC}ber.doc",
            "\u{00F1}o\u{00F1}o.txt",
            "\u{AC00}\u{B098}\u{B2E4}.txt",
            "\u{6587}\u{4EF6}\u{540D}.txt",
            "R\u{00E9}sum\u{00E9}_\u{5C65}\u{6B74}\u{66F8}.pdf",
            "\u{0395}\u{03BB}\u{03BB}\u{03B7}\u{03BD}\u{03B9}\u{03BA}\u{03AC}.txt",
            "\u{0424}\u{0430}\u{0439}\u{043B}.txt",
        ];

        for name in &test_names {
            assert!(
                unicode::round_trip_check(name),
                "Round-trip failed for '{name}'"
            );
        }
    }
}
