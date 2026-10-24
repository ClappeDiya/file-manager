//! Bandwidth throttling for transfer operations.
//!
//! A handle on the one shared limiter (`ufop_core::throttle::SharedRate`,
//! also used by backups and sync). The transfer queue's limit
//! (`set_global_throttle`) is [`TRANSFER_RATE`]; every chunk is also paced
//! by the machine-wide upload limit, which follows the bandwidth timetable.

use std::time::{Duration, Instant};
use ufop_core::throttle::{SharedRate, GLOBAL_UPLOAD};

/// The transfer queue's limit, shared by every transfer worker.
pub static TRANSFER_RATE: SharedRate = SharedRate::new();

enum Rate {
    Owned(SharedRate),
    Global(&'static SharedRate),
}

/// Bandwidth throttler. When the limit is 0, throttling is disabled.
pub struct BandwidthThrottler {
    rate: Rate,
}

impl BandwidthThrottler {
    /// A throttler of its own with the given bytes-per-second limit.
    /// A limit of 0 means unlimited.
    pub fn new(limit_bps: u64) -> Self {
        let rate = SharedRate::new();
        rate.set(limit_bps);
        Self { rate: Rate::Owned(rate) }
    }

    /// The transfer queue's throttler ([`TRANSFER_RATE`]), which
    /// [`pace_chunk`] applies to every transferred chunk.
    pub fn global() -> Self {
        Self { rate: Rate::Global(&TRANSFER_RATE) }
    }

    fn shared(&self) -> &SharedRate {
        match &self.rate {
            Rate::Owned(r) => r,
            Rate::Global(r) => r,
        }
    }

    /// Update the bandwidth limit dynamically.
    pub fn set_limit(&self, limit_bps: u64) {
        self.shared().set(limit_bps);
    }

    /// Get the current limit.
    pub fn limit(&self) -> u64 {
        self.shared().base()
    }

    /// Reserve `bytes` on the time line. Returns the bytes reserved (all of
    /// them) and how long to wait before sending them.
    pub async fn acquire(&self, bytes: u64) -> (u64, Duration) {
        let wait = self.shared().reserve(bytes as usize, Instant::now());
        (bytes, wait)
    }

    /// Wait for the share of the time line `bytes` need. Returns `bytes`.
    pub async fn wait_for(&self, bytes: u64) -> u64 {
        let (allowed, wait) = self.acquire(bytes).await;
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }
        allowed
    }
}

/// Pace one transferred chunk of `bytes` that started at `started`: by the
/// transfer queue's limit and the machine-wide upload limit (timetable).
pub async fn pace_chunk(bytes: usize, started: Instant) {
    let wait = TRANSFER_RATE.reserve(bytes, started).max(GLOBAL_UPLOAD.reserve(bytes, started));
    if !wait.is_zero() {
        tokio::time::sleep(wait).await;
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
    async fn chunks_are_paced_by_the_transfer_limit() {
        // Regression: the transfer throttle was configured but never
        // applied to the chunks the pipeline copies.
        let started = Instant::now();
        let t = BandwidthThrottler::new(100_000);
        let (_, first) = t.acquire(10_000).await;
        let (_, second) = t.acquire(10_000).await;
        assert!(second >= Duration::from_millis(150), "{first:?} {second:?}");
        let waited = t.wait_for(1_000).await;
        assert_eq!(waited, 1_000);
        assert!(started.elapsed() >= Duration::from_millis(150));
    }

    #[tokio::test]
    async fn test_wait_for_returns_bytes() {
        let throttler = BandwidthThrottler::new(0);
        let allowed = throttler.wait_for(5000).await;
        assert_eq!(allowed, 5000);
    }
}
