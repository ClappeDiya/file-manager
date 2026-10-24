//! User-saved filter presets (schema v33 `user_filter_presets`): named lists
//! of ordered include/exclude rules, shared by sync pairs and backup jobs.

use crate::core::error::AppError;
use crate::storage::Repository;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use ufop_core::filter_rules::{FilterRule, RuleSet};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserFilterPreset {
    pub id: String,
    pub name: String,
    pub rules: Vec<FilterRule>,
    pub created_at: String,
}

/// Every saved preset, by name.
pub async fn list(repo: &Repository) -> Result<Vec<UserFilterPreset>, AppError> {
    repo.pool()
        .execute(|conn| {
            let mut st = conn.prepare("SELECT id, name, rules_json, created_at FROM user_filter_presets ORDER BY name COLLATE NOCASE, created_at")?;
            let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?)))?;
            let mut out = Vec::new();
            for row in rows {
                let (id, name, rules_json, created_at) = row?;
                // A row we cannot read is skipped rather than failing the list.
                if let Ok(rules) = serde_json::from_str(&rules_json) {
                    out.push(UserFilterPreset { id, name, rules, created_at });
                }
            }
            Ok(out)
        })
        .await
}

/// Create (no `id`) or replace a preset. The rules must compile.
pub async fn save(repo: &Repository, id: Option<String>, name: String, rules: Vec<FilterRule>) -> Result<UserFilterPreset, AppError> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(AppError::validation("Give the filter preset a name"));
    }
    if rules.is_empty() {
        return Err(AppError::validation("A filter preset needs at least one rule"));
    }
    RuleSet::compile(&rules)?;
    let preset = UserFilterPreset {
        id: id.filter(|i| !i.trim().is_empty()).unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        name,
        rules,
        created_at: Utc::now().to_rfc3339(),
    };
    let p = preset.clone();
    let json = serde_json::to_string(&p.rules)?;
    repo.pool()
        .execute(move |conn| {
            conn.execute(
                "INSERT INTO user_filter_presets (id, name, rules_json, created_at) VALUES (?1, ?2, ?3, ?4) \
                 ON CONFLICT(id) DO UPDATE SET name = excluded.name, rules_json = excluded.rules_json",
                rusqlite::params![p.id, p.name, json, p.created_at],
            )?;
            Ok(())
        })
        .await?;
    Ok(preset)
}

/// Delete a preset. Returns false when there was none with that id.
pub async fn delete(repo: &Repository, id: String) -> Result<bool, AppError> {
    repo.pool().execute(move |conn| Ok(conn.execute("DELETE FROM user_filter_presets WHERE id = ?1", [id])? > 0)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use ufop_core::filter_rules::{FilterAction, FilterKind};

    fn rule(p: &str) -> FilterRule {
        FilterRule { action: FilterAction::Exclude, kind: FilterKind::Rclone, pattern: p.into() }
    }

    #[tokio::test]
    async fn presets_crud() {
        let repo = Repository::open_in_memory().await.unwrap();
        assert!(list(&repo).await.unwrap().is_empty());
        let a = save(&repo, None, "  No temp files ".into(), vec![rule("*.tmp")]).await.unwrap();
        assert_eq!(a.name, "No temp files");
        let b = save(&repo, None, "Archives".into(), vec![FilterRule { action: FilterAction::Exclude, kind: FilterKind::Group, pattern: "archives".into() }])
            .await
            .unwrap();
        assert_eq!(list(&repo).await.unwrap().iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), vec!["Archives", "No temp files"]);
        // Update in place.
        save(&repo, Some(a.id.clone()), "No temp or logs".into(), vec![rule("*.tmp"), rule("*.log")]).await.unwrap();
        let all = list(&repo).await.unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all.iter().find(|p| p.id == a.id).unwrap().rules.len(), 2);
        // Invalid input is refused.
        assert!(save(&repo, None, "".into(), vec![rule("*.x")]).await.is_err());
        assert!(save(&repo, None, "Empty".into(), vec![]).await.is_err());
        assert!(save(&repo, None, "Bad".into(), vec![FilterRule { action: FilterAction::Exclude, kind: FilterKind::Regex, pattern: "(".into() }]).await.is_err());
        assert!(delete(&repo, b.id.clone()).await.unwrap());
        assert!(!delete(&repo, b.id).await.unwrap());
        assert_eq!(list(&repo).await.unwrap().len(), 1);
    }
}
