//! Per-sender message rate limit (MUS-9).
//!
//! Each sending session has a token bucket: a burst of [`BURST`] messages,
//! refilled at one every [`REFILL`]. A session that floods gets a clear error,
//! and every other session keeps its own allowance. A broadcast counts as one
//! message, however many members it reaches.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Messages a session may send at once.
pub const BURST: f64 = 20.0;
/// How often a session gets one more message back, up to [`BURST`]: 30 a minute.
pub const REFILL: Duration = Duration::from_secs(2);
/// Buckets untouched this long are full again, so they're dropped.
const IDLE: Duration = Duration::from_secs(10 * 60);

static BUCKETS: Mutex<Option<HashMap<String, (f64, Instant)>>> = Mutex::new(None);

/// Take one message from `sender`'s allowance, or say how long until it has one.
pub fn take(sender: &str) -> Result<(), String> {
    take_at(sender, Instant::now())
}

fn take_at(sender: &str, now: Instant) -> Result<(), String> {
    let mut guard = BUCKETS.lock().unwrap_or_else(|e| e.into_inner());
    let buckets = guard.get_or_insert_with(HashMap::new);
    if buckets.len() > 1024 {
        buckets.retain(|_, (_, at)| now.duration_since(*at) < IDLE);
    }
    let (tokens, at) = buckets.entry(sender.to_string()).or_insert((BURST, now));
    let refilled = now.duration_since(*at).as_secs_f64() / REFILL.as_secs_f64();
    *tokens = (*tokens + refilled).min(BURST);
    *at = now;
    if *tokens >= 1.0 {
        *tokens -= 1.0;
        return Ok(());
    }
    let wait = ((1.0 - *tokens) * REFILL.as_secs_f64()).ceil();
    Err(format!(
        "too many messages from this session: the limit is {} at once and {} a minute; \
         try again in {wait:.0} s",
        BURST as u32,
        (60.0 / REFILL.as_secs_f64()) as u32
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flooding_sender_is_throttled_and_others_are_not() {
        let t0 = Instant::now();
        for _ in 0..BURST as u32 {
            take_at("rate-test-flood", t0).unwrap();
        }
        let refused = take_at("rate-test-flood", t0).unwrap_err();
        assert!(refused.contains("too many messages"), "{refused}");
        take_at("rate-test-other", t0).expect("another sender keeps its allowance");
        take_at("rate-test-flood", t0 + REFILL).expect("one comes back after REFILL");
        assert!(take_at("rate-test-flood", t0 + REFILL).is_err());
    }
}
