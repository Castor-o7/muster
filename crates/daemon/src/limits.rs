//! Connection caps (MUS-9).
//!
//! Each listener counts the connections it holds open, per key, and refuses a
//! new one past its cap. The TLS gate keys on the peer's IP before the token
//! check and on the machine's name after it, so one machine flooding the gate
//! is turned away while other machines keep connecting. The local gate and
//! the hook listener, which only the daemon's own host can reach, have one
//! cap each. Every cap can be changed with an environment variable.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Counts open connections per key, up to `max` each.
pub struct ConnLimiter {
    max: usize,
    open: Mutex<HashMap<String, usize>>,
}

/// Holds one connection's place; dropping it frees the place.
pub struct ConnGuard {
    limiter: Arc<ConnLimiter>,
    key: String,
}

impl ConnLimiter {
    pub fn new(max: usize) -> Arc<Self> {
        Arc::new(Self {
            max,
            open: Mutex::new(HashMap::new()),
        })
    }

    /// A limiter whose cap comes from `var` when that is set to a number.
    pub fn from_env(var: &str, default: usize) -> Arc<Self> {
        let max = std::env::var(var)
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .filter(|n| *n > 0)
            .unwrap_or(default);
        Self::new(max)
    }

    /// Take a place for `key`, or `None` when `key` already holds `max`.
    pub fn try_acquire(self: &Arc<Self>, key: &str) -> Option<ConnGuard> {
        let mut open = self.open.lock().unwrap_or_else(|e| e.into_inner());
        let count = open.entry(key.to_string()).or_insert(0);
        if *count >= self.max {
            return None;
        }
        *count += 1;
        Some(ConnGuard {
            limiter: self.clone(),
            key: key.to_string(),
        })
    }

    pub fn max(&self) -> usize {
        self.max
    }
}

impl Drop for ConnGuard {
    fn drop(&mut self) {
        let mut open = self.limiter.open.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(count) = open.get_mut(&self.key) {
            *count -= 1;
            if *count == 0 {
                open.remove(&self.key);
            }
        }
    }
}

/// Connections one peer IP may hold on the TLS gate, authenticated or not.
pub const PER_IP: (&str, usize) = ("MUSTER_MAX_CONN_PER_IP", 128);
/// Connections one paired machine may hold on the TLS gate.
pub const PER_MACHINE: (&str, usize) = ("MUSTER_MAX_CONN_PER_MACHINE", 64);
/// Connections the local gate may hold.
pub const LOCAL: (&str, usize) = ("MUSTER_MAX_CONN_LOCAL", 256);
/// Connections the hook listener may hold.
pub const HOOKS: (&str, usize) = ("MUSTER_MAX_CONN_HOOKS", 256);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_past_its_cap_is_refused_and_others_are_not() {
        let limiter = ConnLimiter::new(2);
        let a1 = limiter.try_acquire("flood").unwrap();
        let _a2 = limiter.try_acquire("flood").unwrap();
        assert!(limiter.try_acquire("flood").is_none(), "third is refused");
        assert!(limiter.try_acquire("other").is_some(), "another key isn't");
        drop(a1);
        assert!(
            limiter.try_acquire("flood").is_some(),
            "a closed one frees a place"
        );
    }
}
