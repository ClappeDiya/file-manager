//! Bandwidth throttling for transfer operations.
//!
//! Provides a token-bucket based throttler that limits transfer speed
//! to a configurable bytes-per-second rate.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

/// Token-bucket bandwidth throttler.
///
/// When `limit_bps` is 0, throttling is disabled (unlimited speed).
/// Otherwise, the throttler enforces the specified bytes-per-second rate.
pub struct BandwidthThrottler {
    /// Bytes per second limit (0 = unlimited).
    limit_bps: AtomicU64,
    /// Internal state for token bucket.
    state: Mutex<ThrottleState>,
}

struct ThrottleState {
    /// Available tokens (bytes that can be transferred immediately).
    tokens: f64,
    /// Last time tokens were refilled.
    last_refill: Instant,
}

impl BandwidthThrottler {
    /// Create a new throttler with the given bytes-per-second limit.
    /// A limit of 0 means unlimited.
    pub fn new(limit_bps: u64) -> Self {
        Self {
            limit_bps: AtomicU64::new(limit_bps),
            state: Mutex::new(ThrottleState {
                tokens: limit_bps as f64,
                last_refill: Instant::now(),
            }),
        }
    }

    /// Update the bandwidth limit dynamically.
    pub fn set_limit(&self, limit_bps: u64) {
        self.limit_bps.store(limit_bps, Ordering::Relaxed);
    }

    /// Get the current limit.
    pub fn limit(&self) -> u64 {
        self.limit_bps.load(Ordering::Relaxed)
    }

    /// Request permission to transfer `bytes` amount of data.
    /// Returns the number of bytes actually allowed (may be less than requested)
    /// and the duration to wait before the next request.
    ///
    /// If throttling is disabled (limit = 0), returns the full requested amount
    /// with zero wait time.
    pub async fn acquire(&self, bytes: u64) -> (u64, Duration) {
        let limit = self.limit_bps.load(Ordering::Relaxed);
        if limit == 0 {
            return (bytes, Duration::ZERO);
        }

        let mut state = self.state.lock().await;
        let now = Instant::now();
        let elapsed = now.duration_since(state.last_refill);

        // Refill tokens based on elapsed time
        let refill = (elapsed.as_secs_f64()) * (limit as f64);
        state.tokens = (state.tokens + refill).min(limit as f64);
        state.last_refill = now;

        if state.tokens >= bytes as f64 {
            // Enough tokens available
            state.tokens -= bytes as f64;
            (bytes, Duration::ZERO)
        } else if state.tokens >= 1.0 {
            // Partial tokens available
            let allowed = state.tokens as u64;
            state.tokens = 0.0;
            let wait = Duration::from_secs_f64((bytes - allowed) as f64 / limit as f64);
            (allowed, wait)
        } else {
            // No tokens available, calculate wait time
            let wait = Duration::from_secs_f64(bytes as f64 / limit as f64);
            (0, wait)
        }
    }

    /// Wait until the throttler allows at least `min_bytes` to be transferred.
    /// Returns the number of bytes allowed.
    pub async fn wait_for(&self, bytes: u64) -> u64 {
        let limit = self.limit_bps.load(Ordering::Relaxed);
        if limit == 0 {
            return bytes;
        }

        loop {
            let (allowed, wait) = self.acquire(bytes).await;
            if allowed > 0 {
                return allowed;
            }
            tokio::time::sleep(wait.min(Duration::from_millis(100))).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_unlimited_throttle() {
        let throttler = BandwidthThrottler::new(0);
        let (allowed, wait) = throttler.acquire(1_000_000).await;
        assert_eq!(allowed, 1_000_000);
        assert_eq!(wait, Duration::ZERO);
    }

    #[tokio::test]
    async fn test_limited_throttle_initial_burst() {
        // 1 MB/s limit
        let throttler = BandwidthThrottler::new(1_000_000);

        // Should allow initial burst up to the limit
        let (allowed, _wait) = throttler.acquire(500_000).await;
        assert_eq!(allowed, 500_000);
    }

    #[tokio::test]
    async fn test_throttle_depleted() {
        let throttler = BandwidthThrottler::new(1000);

        // Deplete tokens
        let (allowed, _) = throttler.acquire(1000).await;
        assert_eq!(allowed, 1000);

        // Next request should get 0 or partial
        let (allowed, wait) = throttler.acquire(1000).await;
        // Tokens should be depleted or nearly so
        assert!(allowed < 1000 || wait > Duration::ZERO);
    }

    #[tokio::test]
    async fn test_set_limit_dynamic() {
        let throttler = BandwidthThrottler::new(1000);
        assert_eq!(throttler.limit(), 1000);

        throttler.set_limit(2000);
        assert_eq!(throttler.limit(), 2000);

        throttler.set_limit(0);
        assert_eq!(throttler.limit(), 0);

        // Unlimited should pass everything through
        let (allowed, wait) = throttler.acquire(1_000_000).await;
        assert_eq!(allowed, 1_000_000);
        assert_eq!(wait, Duration::ZERO);
    }

    #[tokio::test]
    async fn test_wait_for_returns_bytes() {
        let throttler = BandwidthThrottler::new(0);
        let allowed = throttler.wait_for(5000).await;
        assert_eq!(allowed, 5000);
    }
}
