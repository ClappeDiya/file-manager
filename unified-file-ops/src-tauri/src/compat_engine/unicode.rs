//! Unicode normalizer for cross-platform filename compatibility (T-034).
//!
//! Handles NFC/NFD conversion based on source/destination OS:
//! - macOS HFS+/APFS: stores filenames in NFD (decomposed)
//! - Windows NTFS: stores filenames in NFC (composed)
//! - Linux ext4: stores raw bytes (no normalization)
//! - Cloud services: generally expect NFC
//!
//! Handles Hangul jamo, combining diacriticals, zero-width joiners,
//! and other Unicode edge cases.

use unicode_normalization::UnicodeNormalization;

/// Unicode normalization form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NormForm {
    /// Canonical Decomposition, followed by Canonical Composition (NFC).
    /// Used by Windows, most cloud services, and Linux conventions.
    Nfc,
    /// Canonical Decomposition (NFD).
    /// Used by macOS HFS+ and APFS.
    Nfd,
}

/// Result of a unicode normalization operation.
#[derive(Debug, Clone)]
pub struct NormResult {
    pub original: String,
    pub normalized: String,
    pub form_applied: NormForm,
    pub was_modified: bool,
    /// Details about what changed.
    pub changes: Vec<String>,
}

/// Normalize a filename to the specified form.
pub fn normalize(name: &str, target_form: NormForm) -> NormResult {
    let normalized = match target_form {
        NormForm::Nfc => name.nfc().collect::<String>(),
        NormForm::Nfd => name.nfd().collect::<String>(),
    };

    let was_modified = normalized != name;
    let mut changes = Vec::new();

    if was_modified {
        changes.push(format!(
            "Normalized to {:?}: {} codepoints -> {} codepoints",
            target_form,
            name.chars().count(),
            normalized.chars().count()
        ));

        // Detect specific categories of changes
        if has_hangul_jamo(name) {
            changes.push("Hangul jamo composition/decomposition applied".to_string());
        }
        if has_combining_diacriticals(name) || has_combining_diacriticals(&normalized) {
            changes.push("Combining diacriticals adjusted".to_string());
        }
    }

    NormResult {
        original: name.to_string(),
        normalized,
        form_applied: target_form,
        was_modified,
        changes,
    }
}

/// Determine the preferred normalization form for a target platform/profile.
pub fn preferred_form(target: &str) -> NormForm {
    match target {
        "macos" | "macos-apfs-ci" | "macos-apfs-cs" | "macos-hfs" => NormForm::Nfd,
        _ => NormForm::Nfc,
    }
}

/// Check if a string is already in NFC form.
pub fn is_nfc(s: &str) -> bool {
    unicode_normalization::is_nfc(s)
}

/// Check if a string is already in NFD form.
pub fn is_nfd(s: &str) -> bool {
    unicode_normalization::is_nfd(s)
}

/// Round-trip a filename through NFC->NFD or NFD->NFC and verify it matches.
/// Returns true if the round-trip preserves the string (via normalization mapping).
pub fn round_trip_check(name: &str) -> bool {
    let nfc = name.nfc().collect::<String>();
    let nfd = nfc.nfd().collect::<String>();
    let back_to_nfc = nfd.nfc().collect::<String>();
    nfc == back_to_nfc
}

/// Remove zero-width characters that may cause display issues.
pub fn strip_zero_width(name: &str) -> (String, Vec<String>) {
    let mut changes = Vec::new();
    let mut result = String::with_capacity(name.len());

    for ch in name.chars() {
        match ch {
            '\u{200B}' => {
                // Zero-width space
                if !changes.iter().any(|c: &String| c.contains("zero-width space")) {
                    changes.push("Removed zero-width space (U+200B)".to_string());
                }
            }
            '\u{200C}' => {
                // Zero-width non-joiner
                if !changes.iter().any(|c: &String| c.contains("non-joiner")) {
                    changes.push("Removed zero-width non-joiner (U+200C)".to_string());
                }
            }
            '\u{200D}' => {
                // Zero-width joiner -- keep for emoji sequences, remove for filenames
                if !changes.iter().any(|c: &String| c.contains("joiner (U+200D)")) {
                    changes.push("Removed zero-width joiner (U+200D)".to_string());
                }
            }
            '\u{FEFF}' => {
                // BOM / zero-width no-break space
                if !changes.iter().any(|c: &String| c.contains("BOM")) {
                    changes.push("Removed BOM/zero-width no-break space (U+FEFF)".to_string());
                }
            }
            '\u{2060}' => {
                // Word joiner
                if !changes.iter().any(|c: &String| c.contains("word joiner")) {
                    changes.push("Removed word joiner (U+2060)".to_string());
                }
            }
            _ => {
                result.push(ch);
            }
        }
    }

    (result, changes)
}

/// Remove bidirectional override characters that could be used for spoofing.
pub fn strip_bidi_overrides(name: &str) -> (String, Vec<String>) {
    let mut changes = Vec::new();
    let mut result = String::with_capacity(name.len());

    for ch in name.chars() {
        match ch {
            '\u{202A}' | '\u{202B}' | '\u{202C}' | '\u{202D}' | '\u{202E}'
            | '\u{2066}' | '\u{2067}' | '\u{2068}' | '\u{2069}' => {
                if changes.is_empty() {
                    changes.push("Removed bidirectional override characters".to_string());
                }
            }
            _ => {
                result.push(ch);
            }
        }
    }

    (result, changes)
}

// -- Internal helpers --

fn has_hangul_jamo(s: &str) -> bool {
    s.chars().any(|c| {
        let cp = c as u32;
        // Hangul Jamo: U+1100..U+11FF
        // Hangul Compatibility Jamo: U+3130..U+318F
        // Hangul Syllables: U+AC00..U+D7AF
        (0x1100..=0x11FF).contains(&cp)
            || (0x3130..=0x318F).contains(&cp)
            || (0xAC00..=0xD7AF).contains(&cp)
    })
}

fn has_combining_diacriticals(s: &str) -> bool {
    s.chars().any(|c| {
        let cp = c as u32;
        // Combining Diacritical Marks: U+0300..U+036F
        // Combining Diacritical Marks Extended: U+1AB0..U+1AFF
        // Combining Diacritical Marks Supplement: U+1DC0..U+1DFF
        (0x0300..=0x036F).contains(&cp)
            || (0x1AB0..=0x1AFF).contains(&cp)
            || (0x1DC0..=0x1DFF).contains(&cp)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Basic NFC/NFD conversion ──

    #[test]
    fn test_nfc_to_nfd() {
        // e-acute as single codepoint (NFC) -> e + combining acute (NFD)
        let nfc = "\u{00E9}"; // e with acute (composed)
        let result = normalize(nfc, NormForm::Nfd);
        assert!(result.was_modified);
        assert_eq!(result.normalized, "e\u{0301}");
    }

    #[test]
    fn test_nfd_to_nfc() {
        // e + combining acute (NFD) -> e-acute (NFC)
        let nfd = "e\u{0301}";
        let result = normalize(nfd, NormForm::Nfc);
        assert!(result.was_modified);
        assert_eq!(result.normalized, "\u{00E9}");
    }

    #[test]
    fn test_already_nfc() {
        let nfc = "\u{00E9}";
        let result = normalize(nfc, NormForm::Nfc);
        assert!(!result.was_modified);
    }

    #[test]
    fn test_already_nfd() {
        let nfd = "e\u{0301}";
        let result = normalize(nfd, NormForm::Nfd);
        assert!(!result.was_modified);
    }

    // ── Round-trip preservation ──

    #[test]
    fn test_round_trip_latin_accents() {
        let names = [
            "\u{00E9}", "\u{00F1}", "\u{00FC}", "\u{00E0}",
            "\u{00C9}", "\u{00D6}", "\u{00C7}",
        ];
        for name in &names {
            assert!(round_trip_check(name), "Round-trip failed for {name}");
        }
    }

    #[test]
    fn test_round_trip_hangul() {
        let hangul = "\u{AC00}\u{B098}\u{B2E4}"; // 가나다
        assert!(round_trip_check(hangul));
    }

    // ── is_nfc / is_nfd checks ──

    #[test]
    fn test_is_nfc() {
        assert!(is_nfc("\u{00E9}"));
        assert!(!is_nfc("e\u{0301}"));
    }

    #[test]
    fn test_is_nfd() {
        assert!(is_nfd("e\u{0301}"));
        // NFC may also be valid NFD for some chars
    }

    // ── Zero-width stripping ──

    #[test]
    fn test_strip_zero_width_space() {
        let (result, changes) = strip_zero_width("file\u{200B}name.txt");
        assert_eq!(result, "filename.txt");
        assert!(!changes.is_empty());
    }

    #[test]
    fn test_strip_zero_width_joiner() {
        let (result, changes) = strip_zero_width("test\u{200D}file.txt");
        assert_eq!(result, "testfile.txt");
        assert!(!changes.is_empty());
    }

    #[test]
    fn test_strip_bom() {
        let (result, changes) = strip_zero_width("\u{FEFF}file.txt");
        assert_eq!(result, "file.txt");
        assert!(!changes.is_empty());
    }

    // ── Bidi override stripping ──

    #[test]
    fn test_strip_bidi_overrides() {
        // RLO (right-to-left override) used in filename spoofing
        let (result, changes) = strip_bidi_overrides("file\u{202E}txt.exe");
        assert_eq!(result, "filetxt.exe");
        assert!(!changes.is_empty());
    }

    // ── Preferred form for platforms ──

    #[test]
    fn test_preferred_form_macos() {
        assert_eq!(preferred_form("macos"), NormForm::Nfd);
        assert_eq!(preferred_form("macos-apfs-ci"), NormForm::Nfd);
    }

    #[test]
    fn test_preferred_form_windows() {
        assert_eq!(preferred_form("windows"), NormForm::Nfc);
        assert_eq!(preferred_form("windows-ntfs"), NormForm::Nfc);
    }

    #[test]
    fn test_preferred_form_linux() {
        assert_eq!(preferred_form("linux"), NormForm::Nfc);
    }

    #[test]
    fn test_preferred_form_cloud() {
        assert_eq!(preferred_form("onedrive"), NormForm::Nfc);
        assert_eq!(preferred_form("google-drive"), NormForm::Nfc);
        assert_eq!(preferred_form("s3"), NormForm::Nfc);
    }

    // ── Combining diacriticals ──

    #[test]
    fn test_combining_diacriticals_detection() {
        assert!(has_combining_diacriticals("e\u{0301}"));
        assert!(!has_combining_diacriticals("\u{00E9}"));
    }

    // ── Hangul jamo detection ──

    #[test]
    fn test_hangul_jamo_detection() {
        assert!(has_hangul_jamo("\u{AC00}")); // 가
        assert!(has_hangul_jamo("\u{1100}")); // ᄀ (jamo)
        assert!(!has_hangul_jamo("abc"));
    }

    // ── Complex multi-script filenames ──

    #[test]
    fn test_cjk_filename_nfc() {
        let name = "\u{6587}\u{4EF6}\u{540D}.txt"; // 文件名.txt
        let result = normalize(name, NormForm::Nfc);
        // CJK ideographs are stable under normalization
        assert!(!result.was_modified);
    }

    #[test]
    fn test_emoji_filename() {
        let name = "photo_\u{1F600}.jpg";
        let result = normalize(name, NormForm::Nfc);
        assert!(!result.was_modified);
    }

    #[test]
    fn test_arabic_filename() {
        let name = "\u{0645}\u{0644}\u{0641}.txt"; // ملف.txt
        let result = normalize(name, NormForm::Nfc);
        // Arabic letters are stable under normalization
        assert!(!result.was_modified);
    }

    #[test]
    fn test_hebrew_filename() {
        let name = "\u{05E7}\u{05D5}\u{05D1}\u{05E5}.txt"; // קובץ.txt
        let result = normalize(name, NormForm::Nfc);
        assert!(!result.was_modified);
    }

    #[test]
    fn test_mixed_script_filename() {
        let name = "R\u{00E9}sum\u{00E9}_\u{5C65}\u{6B74}\u{66F8}.pdf";
        assert!(round_trip_check(name));
    }
}
