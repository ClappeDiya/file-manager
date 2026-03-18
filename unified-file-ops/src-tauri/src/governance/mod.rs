//! Governance - RBAC, policy engine, audit logging, approval workflows.
//!
//! Provides `AuditLogger` implementing `AuditOperations`.

use crate::core::error::AppError;
use crate::core::traits::{AuditOperations, BoxFuture};
use crate::core::types::AuditEvent;
use std::sync::Arc;
use tokio::sync::RwLock;

/// In-memory audit logger. For production, events are also persisted
/// to SQLite via the storage module.
pub struct AuditLogger {
    events: Arc<RwLock<Vec<AuditEvent>>>,
}

impl AuditLogger {
    pub fn new() -> Self {
        Self {
            events: Arc::new(RwLock::new(Vec::new())),
        }
    }
}

impl Default for AuditLogger {
    fn default() -> Self {
        Self::new()
    }
}

impl AuditOperations for AuditLogger {
    fn log_event(&self, event: AuditEvent) -> BoxFuture<'_, Result<(), AppError>> {
        Box::pin(async move {
            tracing::info!(
                event_type = %event.event_type,
                actor = %event.actor,
                resource = %event.resource,
                "Audit event"
            );
            let mut events = self.events.write().await;
            events.push(event);
            Ok(())
        })
    }

    fn query_events(
        &self,
        limit: u32,
        offset: u32,
    ) -> BoxFuture<'_, Result<Vec<AuditEvent>, AppError>> {
        Box::pin(async move {
            let events = self.events.read().await;
            let start = (offset as usize).min(events.len());
            let end = (start + limit as usize).min(events.len());
            Ok(events[start..end].to_vec())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use uuid::Uuid;

    fn make_event(event_type: &str) -> AuditEvent {
        AuditEvent {
            id: Uuid::new_v4(),
            timestamp: Utc::now(),
            event_type: event_type.to_string(),
            actor: "test_user".to_string(),
            resource: "/tmp/test".to_string(),
            details: "Test event".to_string(),
        }
    }

    #[tokio::test]
    async fn test_log_and_query() {
        let logger = AuditLogger::new();
        logger.log_event(make_event("file_copy")).await.unwrap();
        logger.log_event(make_event("file_delete")).await.unwrap();

        let events = logger.query_events(10, 0).await.unwrap();
        assert_eq!(events.len(), 2);
    }

    #[tokio::test]
    async fn test_query_pagination() {
        let logger = AuditLogger::new();
        for i in 0..5 {
            logger
                .log_event(make_event(&format!("event_{i}")))
                .await
                .unwrap();
        }

        let page1 = logger.query_events(2, 0).await.unwrap();
        assert_eq!(page1.len(), 2);

        let page2 = logger.query_events(2, 2).await.unwrap();
        assert_eq!(page2.len(), 2);

        let page3 = logger.query_events(2, 4).await.unwrap();
        assert_eq!(page3.len(), 1);
    }
}
