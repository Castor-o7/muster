//! Message rate limits (MUS-9, MUS-60).
//!
//! Each sending session has a token bucket: a burst of [`BURST`] messages,
//! refilled at one every [`REFILL`]. Each machine has one too, shared by all
//! its sessions: a burst of [`MACHINE_BURST`], refilled at one every
//! [`MACHINE_REFILL`]. A message needs room in both, so a machine can't
//! multiply its rate by registering more sessions. A session that floods gets
//! a clear error, and other machines keep their own allowance. A broadcast
//! counts as one message, however many members it reaches.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Messages a session may send at once.
pub const BURST: f64 = 20.0;
/// How often a session gets one more message back, up to [`BURST`]: 30 a minute.
pub const REFILL: Duration = Duration::from_secs(2);
/// Messages all of one machine's sessions may send at once.
pub const MACHINE_BURST: f64 = 60.0;
/// How often a machine gets one more message back: 60 a minute.
pub const MACHINE_REFILL: Duration = Duration::from_secs(1);
/// Buckets untouched this long are full again, so they're dropped.
const IDLE: Duration = Duration::from_secs(10 * 60);

type Buckets = HashMap<String, (f64, Instant)>;
static BUCKETS: Mutex<Option<Buckets>> = Mutex::new(None);
static ENABLED: AtomicBool = AtomicBool::new(true);

/// Turn the limits off or on. The daemon turns them off in plain mode
/// (MUS-64), for parity with upstream Marshal; they're on by default.
pub fn set_enabled(enabled: bool) {
    ENABLED.store(enabled, Ordering::SeqCst);
}

/// Take one message from `sender`'s allowance and from `machine`'s, or say how
/// long until both have one. A session without a machine is limited per session.
pub fn take(sender: &str, machine: Option<&str>) -> Result<(), String> {
    if !ENABLED.load(Ordering::SeqCst) {
        return Ok(());
    }
    take_at(sender, machine, Instant::now())
}

/// A bucket's tokens after refilling it to `now`.
fn level(buckets: &mut Buckets, key: &str, burst: f64, refill: Duration, now: Instant) -> f64 {
    let (tokens, at) = buckets.entry(key.to_string()).or_insert((burst, now));
    let refilled = now.duration_since(*at).as_secs_f64() / refill.as_secs_f64();
    *tokens = (*tokens + refilled).min(burst);
    *at = now;
    *tokens
}

fn take_at(sender: &str, machine: Option<&str>, now: Instant) -> Result<(), String> {
    let mut guard = BUCKETS.lock().unwrap_or_else(|e| e.into_inner());
    let buckets = guard.get_or_insert_with(HashMap::new);
    if buckets.len() > 1024 {
        buckets.retain(|_, (_, at)| now.duration_since(*at) < IDLE);
    }
    let session_key = format!("session:{sender}");
    let machine_key = machine.map(|m| format!("machine:{m}"));
    let session = level(buckets, &session_key, BURST, REFILL, now);
    let shared = machine_key
        .as_deref()
        .map(|k| level(buckets, k, MACHINE_BURST, MACHINE_REFILL, now));
    if session < 1.0 {
        let wait = ((1.0 - session) * REFILL.as_secs_f64()).ceil();
        return Err(format!(
            "too many messages from this session: the limit is {} at once and {} a minute; \
             try again in {wait:.0} s",
            BURST as u32,
            (60.0 / REFILL.as_secs_f64()) as u32
        ));
    }
    if let Some(shared) = shared
        && shared < 1.0
    {
        let wait = ((1.0 - shared) * MACHINE_REFILL.as_secs_f64()).ceil();
        return Err(format!(
            "too many messages from this machine's sessions together: the limit is {} at once \
             and {} a minute; try again in {wait:.0} s",
            MACHINE_BURST as u32,
            (60.0 / MACHINE_REFILL.as_secs_f64()) as u32
        ));
    }
    if let Some((tokens, _)) = buckets.get_mut(&session_key) {
        *tokens -= 1.0;
    }
    if let Some(k) = machine_key
        && let Some((tokens, _)) = buckets.get_mut(&k)
    {
        *tokens -= 1.0;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flooding_sender_is_throttled_and_others_are_not() {
        let t0 = Instant::now();
        for _ in 0..BURST as u32 {
            take_at("rate-test-flood", None, t0).unwrap();
        }
        let refused = take_at("rate-test-flood", None, t0).unwrap_err();
        assert!(refused.contains("too many messages"), "{refused}");
        take_at("rate-test-other", None, t0).expect("another sender keeps its allowance");
        take_at("rate-test-flood", None, t0 + REFILL).expect("one comes back after REFILL");
        assert!(take_at("rate-test-flood", None, t0 + REFILL).is_err());
    }

    #[test]
    fn new_sessions_share_their_machine_total() {
        let t0 = Instant::now();
        let mut sent = 0;
        // Spread a flood across fresh sessions, each well under its own limit.
        for n in 0.. {
            if take_at(&format!("rate-spread-{n}"), Some("rate-spread-machine"), t0).is_err() {
                break;
            }
            sent += 1;
            assert!(
                sent <= MACHINE_BURST as u32,
                "the machine total didn't hold"
            );
        }
        assert_eq!(sent, MACHINE_BURST as u32);
        let refused = take_at("rate-spread-new", Some("rate-spread-machine"), t0).unwrap_err();
        assert!(refused.contains("this machine"), "{refused}");
        take_at("rate-spread-elsewhere", Some("rate-other-machine"), t0)
            .expect("another machine keeps sending");
    }

    #[test]
    fn a_refused_send_spends_nothing() {
        let t0 = Instant::now();
        for _ in 0..BURST as u32 {
            take_at("rate-nospend", Some("rate-nospend-machine"), t0).unwrap();
        }
        assert!(take_at("rate-nospend", Some("rate-nospend-machine"), t0).is_err());
        // The machine bucket wasn't touched by the refusal: 60 - 20 left.
        for n in 0..(MACHINE_BURST - BURST) as u32 {
            take_at(
                &format!("rate-nospend-{n}"),
                Some("rate-nospend-machine"),
                t0,
            )
            .unwrap();
        }
    }
}
