//! What started the backup operation running on this task, for the run
//! history. The entry points that are not a person pressing Run (the
//! scheduler, the REST/RPC API, the fleet agent) wrap their calls in
//! [`scoped`]; `start_backup` / `start_operation` carry the value into the
//! task they spawn, and the run record reads it with [`current`]. Anything
//! that does not say is a manual run.

use std::future::Future;
use ufop_core::run_trigger::RunTrigger;

tokio::task_local! {
    static CURRENT: RunTrigger;
}

/// The trigger of the enclosing [`scoped`] call, or manual.
pub fn current() -> RunTrigger {
    CURRENT.try_with(|t| *t).unwrap_or_default()
}

/// Runs `fut` with `trigger` as the trigger of every backup operation it starts or runs.
pub fn scoped<F: Future>(trigger: RunTrigger, fut: F) -> impl Future<Output = F::Output> {
    CURRENT.scope(trigger, fut)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn defaults_to_manual_and_nests() {
        assert_eq!(current(), RunTrigger::Manual);
        scoped(RunTrigger::Api, async {
            assert_eq!(current(), RunTrigger::Api);
            scoped(RunTrigger::Scheduled, async { assert_eq!(current(), RunTrigger::Scheduled) }).await;
            assert_eq!(current(), RunTrigger::Api);
        })
        .await;
        assert_eq!(current(), RunTrigger::Manual);
    }
}
