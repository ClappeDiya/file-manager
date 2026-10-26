//! One retry/backoff policy for every engine that retries: backup backends,
//! automation runs, webhook and SIEM delivery. Callers decide *what* is
//! retryable ([`is_retryable`] is the shared default for [`AppError`]s);
//! this module decides *when* to try again.

use crate::error::AppError;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// How many times to retry and how long to wait between attempts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RetryPolicy {
    /// Retries after the first attempt (0 = no retry).
    pub retries: u32,
    /// Delay before the first retry.
    pub delay_ms: u64,
    /// Double the delay after every attempt.
    pub exponential: bool,
    /// Upper bound for any single delay.
    pub max_delay_ms: u64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self { retries: 5, delay_ms: 10_000, exponential: true, max_delay_ms: 300_000 }
    }
}

impl RetryPolicy {
    /// No retries at all.
    pub const NONE: RetryPolicy = RetryPolicy { retries: 0, delay_ms: 0, exponential: false, max_delay_ms: 0 };

    /// Delay before retry number `attempt` (0-based).
    pub fn delay_for(&self, attempt: u32) -> Duration {
        let ms = if self.exponential {
            self.delay_ms.saturating_mul(1u64 << attempt.min(20))
        } else {
            self.delay_ms
        };
        Duration::from_millis(ms.min(self.max_delay_ms))
    }

    /// Whether another try is allowed after `attempts_made` failed attempts.
    pub fn allows_retry(&self, attempts_made: u32) -> bool {
        attempts_made <= self.retries
    }
}

/// Whether an operation that failed with `e` may succeed if tried again.
///
/// Network, transfer and busy-service failures often do. Rejected
/// credentials (401/403, wrong password, revoked token) never do, and
/// retrying them can lock the account; neither do bad input, local
/// file-system problems, wrong passphrases or misconfiguration.
pub fn is_retryable(e: &AppError) -> bool {
    !matches!(
        e,
        AppError::Authentication { .. }
            | AppError::Encryption { .. }
            | AppError::PermissionDenied { .. }
            | AppError::Configuration { .. }
            | AppError::FileOperation { .. }
    )
}

/// Outcome of a retried operation, with how many attempts it took.
#[derive(Debug)]
pub struct Retried<T, E> {
    pub result: Result<T, E>,
    pub attempts: u32,
}

/// Run `op` until it succeeds, `retryable` says no, or retries run out,
/// sleeping on the calling thread between attempts. `on_retry` sees each
/// failure that will be retried (for logging) with the coming delay.
pub fn retry_blocking<T, E>(
    policy: &RetryPolicy,
    mut op: impl FnMut(u32) -> Result<T, E>,
    retryable: impl Fn(&E) -> bool,
    mut on_retry: impl FnMut(u32, &E, Duration),
) -> Retried<T, E> {
    let mut attempt = 0;
    loop {
        match op(attempt) {
            Ok(v) => return Retried { result: Ok(v), attempts: attempt + 1 },
            Err(e) if attempt < policy.retries && retryable(&e) => {
                let d = policy.delay_for(attempt);
                on_retry(attempt, &e, d);
                std::thread::sleep(d);
                attempt += 1;
            }
            Err(e) => return Retried { result: Err(e), attempts: attempt + 1 },
        }
    }
}

/// Async [`retry_blocking`]: waits with `tokio::time::sleep`.
#[cfg(feature = "agent")]
pub async fn retry_async<T, E, F, Fut>(
    policy: &RetryPolicy,
    mut op: F,
    retryable: impl Fn(&E) -> bool,
    mut on_retry: impl FnMut(u32, &E, Duration),
) -> Retried<T, E>
where
    F: FnMut(u32) -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
{
    let mut attempt = 0;
    loop {
        match op(attempt).await {
            Ok(v) => return Retried { result: Ok(v), attempts: attempt + 1 },
            Err(e) if attempt < policy.retries && retryable(&e) => {
                let d = policy.delay_for(attempt);
                on_retry(attempt, &e, d);
                tokio::time::sleep(d).await;
                attempt += 1;
            }
            Err(e) => return Retried { result: Err(e), attempts: attempt + 1 },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delays_double_and_cap() {
        let p = RetryPolicy { retries: 5, delay_ms: 100, exponential: true, max_delay_ms: 350 };
        let ms: Vec<u128> = (0..4).map(|a| p.delay_for(a).as_millis()).collect();
        assert_eq!(ms, vec![100, 200, 350, 350]);
        let flat = RetryPolicy { exponential: false, ..p };
        assert_eq!(flat.delay_for(3).as_millis(), 100);
        assert_eq!(RetryPolicy::NONE.delay_for(0), Duration::ZERO);
    }

    #[test]
    fn retries_until_success_or_exhaustion() {
        let p = RetryPolicy { retries: 3, delay_ms: 0, exponential: false, max_delay_ms: 0 };
        let mut seen = Vec::new();
        let r = retry_blocking(&p, |a| if a < 2 { Err(a) } else { Ok("done") }, |_| true, |a, _, _| seen.push(a));
        assert_eq!((r.result, r.attempts), (Ok("done"), 3));
        assert_eq!(seen, vec![0, 1]);

        let r: Retried<(), u32> = retry_blocking(&p, Err, |_| true, |_, _, _| {});
        assert_eq!((r.result, r.attempts), (Err(3), 4));

        let r: Retried<(), &str> = retry_blocking(&p, |_| Err("fatal"), |e| *e != "fatal", |_, _, _| {});
        assert_eq!(r.attempts, 1, "non-retryable errors stop at once");
    }

    #[test]
    fn rejected_credentials_are_never_retryable() {
        assert!(!is_retryable(&AppError::auth("HTTP 401", "check")));
        assert!(!is_retryable(&AppError::validation("bad")));
        assert!(is_retryable(&AppError::connection("reset", "retry")));
        assert!(is_retryable(&AppError::transfer("cut", "retry")));
        let p = RetryPolicy { retries: 3, delay_ms: 0, exponential: false, max_delay_ms: 0 };
        let r: Retried<(), AppError> = retry_blocking(&p, |_| Err(AppError::auth("401", "x")), is_retryable, |_, _, _| {});
        assert_eq!(r.attempts, 1);
    }

    #[test]
    fn serde_defaults_keep_old_configs_readable() {
        let p: RetryPolicy = serde_json::from_str(r#"{"retries":2}"#).unwrap();
        assert_eq!(p, RetryPolicy { retries: 2, ..RetryPolicy::default() });
    }

    #[cfg(feature = "agent")]
    #[test]
    fn async_variant() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap();
        let p = RetryPolicy { retries: 2, delay_ms: 1, exponential: true, max_delay_ms: 5 };
        let r = rt.block_on(retry_async(&p, |a| async move { if a == 0 { Err("x") } else { Ok(a) } }, |_| true, |_, _, _| {}));
        assert_eq!((r.result, r.attempts), (Ok(1), 2));
    }
}
