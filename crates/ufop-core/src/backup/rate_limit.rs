//! Request-rate limits for destinations: a token bucket per kind of
//! request (list, get, put, delete) and a cap on requests in flight, as a
//! [`Backend`] wrapper. Providers that throttle or bill per request
//! (S3 `SlowDown`, Graph 429s, B2 class B/C transactions) stay under their
//! limits instead of burning retries.
//!
//! Time comes from a [`Clock`] so the limiter is tested deterministically.

use super::backend::{Backend, LockMode, Quota, RemoteFile};
use crate::error::AppError;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// Per-destination request limits. Rates are requests per second
/// (0 = unlimited); `max_concurrent_requests` 0 = unlimited.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RequestLimits {
    pub list_per_sec: f64,
    pub get_per_sec: f64,
    pub put_per_sec: f64,
    pub delete_per_sec: f64,
    pub max_concurrent_requests: u32,
    /// Listings of sharded layouts run this many folder listings at once
    /// (local folders, object-store prefixes); 0 or 1 = one at a time.
    pub list_parallelism: u32,
}

impl RequestLimits {
    /// True when any request is limited (the wrapper is needed).
    pub fn is_limited(&self) -> bool {
        [self.list_per_sec, self.get_per_sec, self.put_per_sec, self.delete_per_sec].iter().any(|r| *r > 0.0) || self.max_concurrent_requests > 0
    }

    pub fn validate(&self) -> Result<(), AppError> {
        for (n, r) in [("list", self.list_per_sec), ("get", self.get_per_sec), ("put", self.put_per_sec), ("delete", self.delete_per_sec)] {
            if !r.is_finite() || r < 0.0 {
                return Err(AppError::validation(format!("The {n} request rate must be zero (unlimited) or positive")));
            }
        }
        if self.list_parallelism > 64 {
            return Err(AppError::validation("Listing parallelism must be 64 or less"));
        }
        Ok(())
    }
}

/// Monotonic time and sleeping.
pub trait Clock: Send + Sync {
    fn now(&self) -> Instant;
    fn sleep(&self, d: Duration);
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
    fn sleep(&self, d: Duration) {
        std::thread::sleep(d)
    }
}

/// Token bucket: `rate` tokens per second, holding at most one second's
/// worth (at least one token). Taking a token that is not there reserves
/// it and returns how long to wait, so concurrent callers queue fairly.
#[derive(Debug)]
pub struct TokenBucket {
    rate: f64,
    burst: f64,
    tokens: f64,
    last: Option<Instant>,
}

impl TokenBucket {
    pub fn new(rate: f64) -> Self {
        let burst = rate.max(1.0);
        Self { rate, burst, tokens: burst, last: None }
    }

    /// Take one token at `now`; the wait before the request may go out.
    pub fn take(&mut self, now: Instant) -> Duration {
        if self.rate <= 0.0 {
            return Duration::ZERO;
        }
        if let Some(last) = self.last {
            let dt = now.saturating_duration_since(last).as_secs_f64();
            self.tokens = (self.tokens + dt * self.rate).min(self.burst);
        }
        self.last = Some(now);
        self.tokens -= 1.0;
        if self.tokens >= 0.0 {
            Duration::ZERO
        } else {
            Duration::from_secs_f64(-self.tokens / self.rate)
        }
    }
}

#[derive(Clone, Copy)]
enum Kind {
    List,
    Get,
    Put,
    Delete,
}

/// Counting gate for requests in flight.
struct Gate {
    max: usize,
    used: Mutex<usize>,
    cv: Condvar,
}

impl Gate {
    fn enter(&self) -> Option<GateGuard<'_>> {
        if self.max == 0 {
            return None;
        }
        let mut n = self.used.lock().unwrap_or_else(|e| e.into_inner());
        while *n >= self.max {
            n = self.cv.wait(n).unwrap_or_else(|e| e.into_inner());
        }
        *n += 1;
        Some(GateGuard(self))
    }
}

struct GateGuard<'a>(&'a Gate);

impl Drop for GateGuard<'_> {
    fn drop(&mut self) {
        let mut n = self.0.used.lock().unwrap_or_else(|e| e.into_inner());
        *n -= 1;
        self.0.cv.notify_one();
    }
}

/// [`Backend`] wrapper applying [`RequestLimits`].
pub struct RateLimitedBackend<B: Backend> {
    inner: B,
    clock: Arc<dyn Clock>,
    buckets: [Mutex<TokenBucket>; 4],
    gate: Gate,
}

impl<B: Backend> RateLimitedBackend<B> {
    pub fn new(inner: B, limits: RequestLimits) -> Self {
        Self::with_clock(inner, limits, Arc::new(SystemClock))
    }

    pub fn with_clock(inner: B, l: RequestLimits, clock: Arc<dyn Clock>) -> Self {
        let b = |r: f64| Mutex::new(TokenBucket::new(r));
        Self {
            inner,
            clock,
            buckets: [b(l.list_per_sec), b(l.get_per_sec), b(l.put_per_sec), b(l.delete_per_sec)],
            gate: Gate { max: l.max_concurrent_requests as usize, used: Mutex::new(0), cv: Condvar::new() },
        }
    }

    fn call<T>(&self, kind: Kind, f: impl FnOnce() -> Result<T, AppError>) -> Result<T, AppError> {
        let wait = self.buckets[kind as usize].lock().unwrap_or_else(|e| e.into_inner()).take(self.clock.now());
        if !wait.is_zero() {
            self.clock.sleep(wait);
        }
        let _g = self.gate.enter();
        f()
    }
}

impl<B: Backend> Backend for RateLimitedBackend<B> {
    fn describe(&self) -> String {
        self.inner.describe()
    }
    fn list(&self) -> Result<Vec<RemoteFile>, AppError> {
        self.call(Kind::List, || self.inner.list())
    }
    fn put(&self, n: &str, d: &[u8]) -> Result<(), AppError> {
        self.call(Kind::Put, || self.inner.put(n, d))
    }
    fn get(&self, n: &str) -> Result<Vec<u8>, AppError> {
        self.call(Kind::Get, || self.inner.get(n))
    }
    fn delete(&self, n: &str) -> Result<(), AppError> {
        self.call(Kind::Delete, || self.inner.delete(n))
    }
    fn create_folder(&self) -> Result<(), AppError> {
        self.call(Kind::Put, || self.inner.create_folder())
    }
    fn rename(&self, a: &str, b: &str) -> Result<(), AppError> {
        self.call(Kind::Put, || self.inner.rename(a, b))
    }
    fn quota(&self) -> Option<Quota> {
        self.inner.quota()
    }
    fn supports_lock(&self) -> bool {
        self.inner.supports_lock()
    }
    fn lock(&self, n: &str, u: chrono::DateTime<chrono::Utc>, m: LockMode) -> Result<(), AppError> {
        self.call(Kind::Put, || self.inner.lock(n, u, m))
    }
    fn supports_subfolders(&self) -> bool {
        self.inner.supports_subfolders()
    }
    fn list_recursive(&self) -> Result<Vec<RemoteFile>, AppError> {
        self.call(Kind::List, || self.inner.list_recursive())
    }
    fn modified(&self, n: &str) -> Result<Option<chrono::DateTime<chrono::Utc>>, AppError> {
        self.call(Kind::Get, || self.inner.modified(n))
    }
    fn is_read_only(&self) -> bool {
        self.inner.is_read_only()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::LocalBackend;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Time that only moves when someone sleeps.
    struct FakeClock {
        t0: Instant,
        elapsed: Mutex<Duration>,
        slept: Mutex<Vec<Duration>>,
    }

    impl FakeClock {
        fn new() -> Arc<Self> {
            Arc::new(Self { t0: Instant::now(), elapsed: Mutex::new(Duration::ZERO), slept: Mutex::new(Vec::new()) })
        }
        fn total_slept(&self) -> f64 {
            self.slept.lock().unwrap().iter().map(Duration::as_secs_f64).sum()
        }
    }

    impl Clock for FakeClock {
        fn now(&self) -> Instant {
            self.t0 + *self.elapsed.lock().unwrap()
        }
        fn sleep(&self, d: Duration) {
            *self.elapsed.lock().unwrap() += d;
            self.slept.lock().unwrap().push(d);
        }
    }

    #[test]
    fn token_bucket_bursts_then_paces() {
        let t = Instant::now();
        let mut b = TokenBucket::new(4.0);
        let waits: Vec<f64> = (0..6).map(|_| b.take(t).as_secs_f64()).collect();
        // Four requests of burst, then one every 1/4 s (reserved in order).
        assert_eq!(&waits[..4], &[0.0; 4]);
        assert!((waits[4] - 0.25).abs() < 1e-9 && (waits[5] - 0.5).abs() < 1e-9, "{waits:?}");
        // After a quiet second the bucket is full again (but not over-full).
        let mut b = TokenBucket::new(2.0);
        b.take(t);
        b.take(t);
        assert_eq!(b.take(t + Duration::from_secs(10)), Duration::ZERO);
        assert_eq!(b.take(t + Duration::from_secs(10)), Duration::ZERO);
        assert!(b.take(t + Duration::from_secs(10)) > Duration::ZERO);
        assert_eq!(TokenBucket::new(0.0).take(t), Duration::ZERO, "0 = unlimited");
    }

    #[test]
    fn limits_each_kind_separately() {
        let d = tempfile::tempdir().unwrap();
        let clock = FakeClock::new();
        let limits = RequestLimits { get_per_sec: 10.0, put_per_sec: 2.0, ..Default::default() };
        assert!(limits.is_limited() && !RequestLimits::default().is_limited());
        let b = RateLimitedBackend::with_clock(LocalBackend::new(d.path()), limits, clock.clone());
        b.put("a", b"1").unwrap();
        b.put("b", b"2").unwrap();
        assert_eq!(clock.total_slept(), 0.0, "burst of 2 puts");
        b.put("c", b"3").unwrap();
        assert!((clock.total_slept() - 0.5).abs() < 1e-9, "third put waits 1/2 s");
        let before = clock.total_slept();
        // 30 gets at 10/s: 10 in the burst, then one per 1/10 s.
        for _ in 0..30 {
            b.get("a").unwrap();
        }
        let got = clock.total_slept() - before;
        assert!((got - 2.0).abs() < 1e-6, "{got}");
        // Lists are not limited at all.
        let before = clock.total_slept();
        for _ in 0..50 {
            b.list().unwrap();
        }
        assert_eq!(clock.total_slept(), before);
        assert!(RequestLimits { put_per_sec: -1.0, ..Default::default() }.validate().is_err());
        assert!(RequestLimits { list_parallelism: 65, ..Default::default() }.validate().is_err());
    }

    struct Probe {
        now: AtomicUsize,
        peak: AtomicUsize,
    }

    impl Backend for Probe {
        fn describe(&self) -> String {
            "probe".into()
        }
        fn list(&self) -> Result<Vec<RemoteFile>, AppError> {
            Ok(vec![])
        }
        fn put(&self, _: &str, _: &[u8]) -> Result<(), AppError> {
            Ok(())
        }
        fn get(&self, _: &str) -> Result<Vec<u8>, AppError> {
            let n = self.now.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(n, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(5));
            self.now.fetch_sub(1, Ordering::SeqCst);
            Ok(vec![])
        }
        fn delete(&self, _: &str) -> Result<(), AppError> {
            Ok(())
        }
        fn create_folder(&self) -> Result<(), AppError> {
            Ok(())
        }
    }

    #[test]
    fn caps_requests_in_flight() {
        for cap in [2u32, 1, 0] {
            let b = RateLimitedBackend::new(Probe { now: AtomicUsize::new(0), peak: AtomicUsize::new(0) }, RequestLimits { max_concurrent_requests: cap, ..Default::default() });
            std::thread::scope(|s| {
                for _ in 0..8 {
                    s.spawn(|| {
                        for _ in 0..4 {
                            b.get("x").unwrap();
                        }
                    });
                }
            });
            let peak = b.inner.peak.load(Ordering::SeqCst);
            assert!(peak >= 1);
            if cap > 0 {
                assert!(peak <= cap as usize, "peak {peak} > cap {cap}");
            }
        }
    }
}
