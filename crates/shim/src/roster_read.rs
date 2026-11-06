//! `muster-shim roster`: print the roster as JSON for other programs.
//!
//! It only subscribes to queries. It registers no session, so reading the roster
//! doesn't change it and the reader doesn't appear on it.
//!
//! Each session's facts are split by how far they can be trusted, against a paired
//! machine. A process on the daemon's own host can write anything until MUS-7.
//!
//! - `observed`: only the daemon can set these. myko overwrites `client_id` from
//!   the connection, and the gate refuses a client's `WakeWatcher` events.
//!   They show a connection or a wait exists, not that the session behind it is
//!   working: a machine can hold either open for its own session.
//! - `attested`: the gate writes these for a paired machine. A session on the
//!   daemon's own host doesn't pass the gate, so there they're only its own claim,
//!   and the row doesn't say which case it is.
//! - `declared`: what the session's own software reports, which a machine can set
//!   to anything for its own sessions. That includes `last_activity_at`, because
//!   the shim can set it.

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use chrono::Utc;
use hyphae::Gettable;
use muster_entities::{
    GetAllSessionNicknames, GetAllSessions, GetAllWakePauses, GetAllWakeWatchers, Session,
    SessionNickname, WakePause, WakeWatcher, wake_state,
};
use myko::client::MykoClient;
use serde_json::{Value, json};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const POLL: Duration = Duration::from_millis(100);
/// How long to let the wake watchers arrive once the roster has. There may be
/// none, so this can't wait for them to appear.
const WATCHERS_SETTLE: Duration = Duration::from_secs(1);

fn view(
    sessions: &[Arc<Session>],
    nicknames: &[Arc<SessionNickname>],
    watchers: &[Arc<WakeWatcher>],
    pauses: &[Arc<WakePause>],
    now_ms: i64,
) -> Value {
    let rows: Vec<Value> = sessions
        .iter()
        .map(|s| {
            let id = s.id.0.as_ref();
            let nickname = nicknames
                .iter()
                .find(|n| n.id.0.as_ref() == id)
                .map(|n| n.nickname.clone())
                .unwrap_or_else(|| muster_entities::nickname(id));
            let waiting_until = watchers
                .iter()
                .find(|w| w.id.0.as_ref() == id && w.until > now_ms)
                .map(|w| w.until);
            json!({
                "session_id": id,
                "nickname": nickname,
                "wake": wake_state(s, watchers, now_ms).as_str(),
                "attested": {
                    "machine": s.host.as_ref().map(|h| h.name.as_str()),
                    "operator": s.operator,
                },
                "observed": {
                    // The row is bound to a connection. It can outlast that
                    // connection until the daemon's sweep reaps it.
                    "bound_to_connection": s.client_id.is_some(),
                    "waiting": waiting_until.is_some(),
                    "waiting_until": waiting_until,
                },
                "declared": {
                    "activity": s.activity,
                    "last_activity_at": s.last_activity_at,
                    "last_tool": s.last_tool,
                    "last_tool_at": s.last_tool_at,
                    "connected_at": s.connected_at,
                    "current_task": s.current_task,
                    "current_task_at": s.current_task_at,
                    "session_name": s.session_name,
                    "kind": s.kind,
                    "project": s.project,
                    "cwd": s.cwd,
                    "git_branch": s.git_branch,
                    "pid": s.pid,
                    "channels_enabled": s.channels_enabled,
                    "harness": s.harness,
                    // MUS-3: a claimed role, which reaches this session by name.
                    "role": s.identity.as_deref().filter(|i| *i != "none"),
                },
            })
        })
        .collect();
    // Observed: only the daemon writes it. While true, no message wakes a
    // session, whatever its `wake` says (MUS-10).
    let wakes_paused = pauses.iter().any(|p| p.paused);
    json!({ "read_at": now_ms, "wakes_paused": wakes_paused, "sessions": rows })
}

pub async fn run() -> Result<()> {
    muster_entities::link();
    let client = MykoClient::new();
    let sessions = client.watch_query::<GetAllSessions>(GetAllSessions {});
    let nicknames = client.watch_query::<GetAllSessionNicknames>(GetAllSessionNicknames {});
    let watchers = client.watch_query::<GetAllWakeWatchers>(GetAllWakeWatchers {});
    let pauses = client.watch_query::<GetAllWakePauses>(GetAllWakePauses {});
    client.set_address(Some(crate::daemon_address().await?.0));

    // An empty result can't be told from one that hasn't arrived, so never report
    // an empty roster as a fact.
    let started = Instant::now();
    while sessions.get().is_empty() {
        if started.elapsed() > CONNECT_TIMEOUT {
            bail!(
                "no roster within {CONNECT_TIMEOUT:?}: the daemon is unreachable or has no sessions"
            );
        }
        tokio::time::sleep(POLL).await;
    }
    let settling = Instant::now();
    while watchers.get().is_empty() && settling.elapsed() < WATCHERS_SETTLE {
        tokio::time::sleep(POLL).await;
    }

    let output = view(
        &sessions.get(),
        &nicknames.get(),
        &watchers.get(),
        &pauses.get(),
        Utc::now().timestamp_millis(),
    );
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use muster_entities::{HostInfo, SessionId, WakeWatcherId};

    fn session() -> Arc<Session> {
        Arc::new(Session {
            id: SessionId(Arc::from("a")),
            client_id: None,
            pid: 7,
            cwd: "/w".into(),
            git_branch: None,
            current_task: Some("anything at all".into()),
            session_name: None,
            activity: Some("idle".into()),
            kind: None,
            connected_at: 1,
            last_activity_at: Some(2),
            last_tool: None,
            last_tool_at: None,
            current_task_at: None,
            operator: Some("op".into()),
            host: Some(HostInfo {
                name: "a1".into(),
                os: String::new(),
                arch: String::new(),
            }),
            project: None,
            channels_enabled: None,
            acks_pushes: None,
            harness: None,
            identity: None,
        })
    }

    #[test]
    fn claims_are_kept_apart_from_what_the_daemon_established() {
        let v = view(&[session()], &[], &[], &[], 100);
        let row = &v["sessions"][0];
        assert_eq!(v["read_at"], 100);
        assert_eq!(row["attested"]["machine"], "a1");
        assert!(row["observed"].get("machine").is_none());
        assert!(row["observed"].get("operator").is_none());
        assert_eq!(row["declared"]["current_task"], "anything at all");
        assert_eq!(row["declared"]["last_activity_at"], 2);
        assert!(row["observed"].get("current_task").is_none());
        assert!(row["observed"].get("last_activity_at").is_none());
    }

    #[test]
    fn waiting_comes_from_a_current_watcher() {
        let watcher = |until| {
            Arc::new(WakeWatcher {
                id: WakeWatcherId(Arc::from("a")),
                until,
            })
        };
        let v = view(&[session()], &[], &[watcher(101)], &[], 100);
        assert_eq!(v["sessions"][0]["wake"], "waiting");
        assert_eq!(v["sessions"][0]["observed"]["waiting_until"], 101);
        let v = view(&[session()], &[], &[watcher(100)], &[], 100);
        assert_eq!(v["sessions"][0]["wake"], "none");
        assert_eq!(v["sessions"][0]["observed"]["waiting"], false);
    }
}
