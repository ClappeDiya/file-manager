//! Condition evaluation for automation rules.
//!
//! Checks file metadata against rule conditions using AND logic.
//! All conditions must match for the rule action to execute.

use super::RuleCondition;
use glob::Pattern;
use std::path::Path;
use std::time::SystemTime;

/// Evaluate all conditions against a file path. Returns true if ALL match (AND).
/// If conditions is empty, always returns true.
pub fn evaluate_conditions(conditions: &[RuleCondition], file_path: &str) -> bool {
    if conditions.is_empty() {
        return true;
    }

    let path = Path::new(file_path);
    let metadata = std::fs::metadata(file_path).ok();

    for condition in conditions {
        if !evaluate_single(condition, path, &metadata) {
            return false;
        }
    }

    true
}

fn evaluate_single(
    condition: &RuleCondition,
    path: &Path,
    metadata: &Option<std::fs::Metadata>,
) -> bool {
    match condition {
        RuleCondition::ExtensionIs { extensions } => {
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();
            extensions
                .iter()
                .any(|e| e.to_lowercase().trim_start_matches('.') == ext)
        }

        RuleCondition::NameMatches { pattern } => {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");
            Pattern::new(pattern)
                .map(|p| p.matches(name))
                .unwrap_or(false)
        }

        RuleCondition::SizeGreaterThan { bytes } => metadata
            .as_ref()
            .map(|m| m.len() > *bytes)
            .unwrap_or(false),

        RuleCondition::SizeLessThan { bytes } => metadata
            .as_ref()
            .map(|m| m.len() < *bytes)
            .unwrap_or(false),

        RuleCondition::CreatedWithin { seconds } => {
            metadata
                .as_ref()
                .and_then(|m| m.created().ok())
                .map(|created| {
                    SystemTime::now()
                        .duration_since(created)
                        .map(|d| d.as_secs() <= *seconds)
                        .unwrap_or(false)
                })
                .unwrap_or(false)
        }

        RuleCondition::PathContains { substring } => {
            path.to_str()
                .map(|p| p.contains(substring.as_str()))
                .unwrap_or(false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_empty_conditions_always_matches() {
        assert!(evaluate_conditions(&[], "/any/path.txt"));
    }

    #[test]
    fn test_extension_match() {
        let conditions = vec![RuleCondition::ExtensionIs {
            extensions: vec!["pdf".to_string(), "doc".to_string()],
        }];
        assert!(evaluate_conditions(&conditions, "/test/report.pdf"));
        assert!(evaluate_conditions(&conditions, "/test/report.PDF")); // case insensitive
        assert!(!evaluate_conditions(&conditions, "/test/report.txt"));
    }

    #[test]
    fn test_extension_with_dot_prefix() {
        let conditions = vec![RuleCondition::ExtensionIs {
            extensions: vec![".pdf".to_string()],
        }];
        assert!(evaluate_conditions(&conditions, "/test/report.pdf"));
    }

    #[test]
    fn test_name_glob_pattern() {
        let conditions = vec![RuleCondition::NameMatches {
            pattern: "report_*".to_string(),
        }];
        assert!(evaluate_conditions(&conditions, "/test/report_2024.pdf"));
        assert!(!evaluate_conditions(&conditions, "/test/invoice_2024.pdf"));
    }

    #[test]
    fn test_path_contains() {
        let conditions = vec![RuleCondition::PathContains {
            substring: "Downloads".to_string(),
        }];
        assert!(evaluate_conditions(&conditions, "/Users/me/Downloads/file.pdf"));
        assert!(!evaluate_conditions(&conditions, "/Users/me/Documents/file.pdf"));
    }

    #[test]
    fn test_size_conditions_with_real_file() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        let mut file = std::fs::File::create(&file_path).unwrap();
        file.write_all(&[0u8; 500]).unwrap(); // 500 bytes

        let path_str = file_path.to_str().unwrap();

        assert!(evaluate_conditions(
            &[RuleCondition::SizeGreaterThan { bytes: 100 }],
            path_str,
        ));
        assert!(!evaluate_conditions(
            &[RuleCondition::SizeGreaterThan { bytes: 1000 }],
            path_str,
        ));
        assert!(evaluate_conditions(
            &[RuleCondition::SizeLessThan { bytes: 1000 }],
            path_str,
        ));
    }

    #[test]
    fn test_and_logic_multiple_conditions() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("report.pdf");
        let mut file = std::fs::File::create(&file_path).unwrap();
        file.write_all(&[0u8; 500]).unwrap();

        let path_str = file_path.to_str().unwrap();

        // Both conditions must match
        let conditions = vec![
            RuleCondition::ExtensionIs {
                extensions: vec!["pdf".to_string()],
            },
            RuleCondition::SizeGreaterThan { bytes: 100 },
        ];
        assert!(evaluate_conditions(&conditions, path_str));

        // One fails -> whole thing fails
        let conditions_fail = vec![
            RuleCondition::ExtensionIs {
                extensions: vec!["txt".to_string()],
            },
            RuleCondition::SizeGreaterThan { bytes: 100 },
        ];
        assert!(!evaluate_conditions(&conditions_fail, path_str));
    }
}
