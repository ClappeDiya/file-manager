//! What started a run: shown in the run history of sync pairs, backup jobs
//! and automation rules, and stored with every sync and backup run.

use serde::{Deserialize, Serialize};

/// The way a run was started.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum RunTrigger {
    /// A person pressed Run (desktop UI, `ufop` CLI in the foreground).
    #[default]
    Manual,
    /// The pair's / job's own schedule came due.
    Scheduled,
    /// A filesystem watcher saw changes.
    Watcher,
    /// Another program: the server's REST/MCP API, the fleet agent, chat control.
    Api,
    /// An automation rule's action.
    Automation,
}

impl RunTrigger {
    pub const ALL: [RunTrigger; 5] = [Self::Manual, Self::Scheduled, Self::Watcher, Self::Api, Self::Automation];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Scheduled => "scheduled",
            Self::Watcher => "watcher",
            Self::Api => "api",
            Self::Automation => "automation",
        }
    }

    /// Parses a stored value; anything unknown is a manual run (the column default).
    pub fn from_str_lossy(s: &str) -> Self {
        Self::ALL.into_iter().find(|t| t.as_str() == s.trim()).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_defaults_to_manual() {
        for t in RunTrigger::ALL {
            assert_eq!(RunTrigger::from_str_lossy(t.as_str()), t);
            assert_eq!(serde_json::to_value(t).unwrap(), serde_json::json!(t.as_str()));
        }
        assert_eq!(RunTrigger::from_str_lossy("whatever"), RunTrigger::Manual);
        assert_eq!(RunTrigger::from_str_lossy(""), RunTrigger::Manual);
    }
}
