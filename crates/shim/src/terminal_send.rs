//! `muster-shim all` and `muster-shim tell`: message the muster from a terminal,
//! outside any Claude session.
//!
//! Sends go through myko like any other, so the sender has to be on the roster.
//! The terminal registers a short-lived `Session` of kind `terminal`, under one id
//! kept per machine so the gate's binding file doesn't grow, and removes it on
//! exit. Each recipient gets a direct message rather than a room broadcast,
//! because only direct mail wakes an idle session.
//!
//! The gate attests the machine, not the person: any process on this account can
//! run this command, so the label says which machine and carries no authority.
//! It is never the operator's approval for a restart or anything similar.

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use chrono::Utc;
use hyphae::Gettable;
use muster_entities::{
    GetAllSessionNicknames, GetAllSessions, LivePushStatus, SendMessage, SendMessageResult,
    Session, SessionId, SessionNickname, WakeStatus,
};
use myko::client::MykoClient;
use uuid::Uuid;

const TERMINAL_KIND: &str = "terminal";
const TERMINAL_ID_FILE: &str = "terminal-session";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const CONNECT_POLL: Duration = Duration::from_millis(200);
const SEND_TIMEOUT: Duration = Duration::from_secs(10);
/// A shortest usable id prefix, so a stray short word can't match a session.
const MIN_ID_PREFIX: usize = 8;

pub enum Target {
    All,
    Named(String),
}

/// The sessions `target` names, leaving out terminals, which can't read mail.
fn resolve_targets(
    sessions: &[Arc<Session>],
    nicknames: &[Arc<SessionNickname>],
    target: &Target,
) -> Result<Vec<Arc<Session>>, String> {
    let agents = sessions
        .iter()
        .filter(|s| s.kind.as_deref() != Some(TERMINAL_KIND));
    let Target::Named(token) = target else {
        return Ok(agents.cloned().collect());
    };
    let agents: Vec<_> = agents.collect();

    let team: Vec<_> = agents
        .iter()
        .filter(|s| {
            s.project
                .as_deref()
                .is_some_and(|p| p.eq_ignore_ascii_case(token))
        })
        .collect();
    let single = agents
        .iter()
        .find(|s| s.id.0.as_ref() == token || handle(nicknames, s) == *token);
    match (team.is_empty(), single) {
        (false, Some(_)) => {
            return Err(format!(
                "`{token}` is both a team and a session; use the session's full id"
            ));
        }
        (false, None) => return Ok(team.into_iter().map(|s| (**s).clone()).collect()),
        (true, Some(session)) => return Ok(vec![(**session).clone()]),
        (true, None) => {}
    }

    let by_prefix: Vec<_> = agents
        .iter()
        .filter(|s| token.len() >= MIN_ID_PREFIX && s.id.0.starts_with(token.as_str()))
        .collect();
    match by_prefix.as_slice() {
        [] => Err(format!("no team or session named `{token}` on the roster")),
        [session] => Ok(vec![(***session).clone()]),
        _ => Err(format!(
            "`{token}` matches {} sessions; use more of the id",
            by_prefix.len()
        )),
    }
}

fn handle(nicknames: &[Arc<SessionNickname>], session: &Session) -> String {
    let id = session.id.0.as_ref();
    nicknames
        .iter()
        .find(|n| n.id.0.as_ref() == id)
        .map(|n| n.nickname.clone())
        .unwrap_or_else(|| muster_entities::nickname(id))
}

fn labelled(machine: &str, body: &str) -> String {
    format!("[from a terminal on {machine}; it can't receive replies] {body}")
}

/// What the sender can honestly say about one send.
fn outcome(result: &SendMessageResult) -> &'static str {
    match (result.effective_live_push(), result.effective_wake()) {
        (LivePushStatus::Delivered, _) => "delivered live",
        (_, WakeStatus::Expected) => "in inbox, wake expected but not observed",
        _ => "in inbox, wake not observed",
    }
}

/// This machine's terminal id, created on first use.
fn terminal_id() -> Result<String> {
    let candidates = crate::config_file_candidates(TERMINAL_ID_FILE);
    for path in &candidates {
        if let Ok(id) = std::fs::read_to_string(path)
            && Uuid::parse_str(id.trim()).is_ok()
        {
            return Ok(id.trim().to_string());
        }
    }
    let path = candidates
        .first()
        .context("no config directory for this machine")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let id = Uuid::new_v4().to_string();
    std::fs::write(path, &id).with_context(|| format!("writing {}", path.display()))?;
    Ok(id)
}

pub async fn run(target: Target, body: String) -> Result<()> {
    muster_entities::link();
    let address = crate::daemon_address().await?;
    let machine = crate::link::load_pairing()
        .map(|p| p.name)
        .unwrap_or_else(|| crate::detect_host().name);
    let cwd = std::env::current_dir()
        .map(|d| d.display().to_string())
        .unwrap_or_default();
    let id = SessionId(Arc::from(terminal_id()?.as_str()));
    let session = Session {
        id: id.clone(),
        client_id: None,
        pid: std::process::id(),
        operator: Some(crate::detect_operator(&cwd)),
        host: Some(crate::detect_host()),
        cwd,
        git_branch: None,
        current_task: None,
        session_name: None,
        activity: None,
        kind: Some(TERMINAL_KIND.to_string()),
        connected_at: Utc::now().timestamp_millis(),
        last_activity_at: Some(Utc::now().timestamp_millis()),
        last_tool: None,
        last_tool_at: None,
        project: None,
        channels_enabled: None,
    };

    let client = Arc::new(MykoClient::new());
    let sessions = client.watch_query::<GetAllSessions>(GetAllSessions {});
    let nicknames = client.watch_query::<GetAllSessionNicknames>(GetAllSessionNicknames {});
    client.set_address(Some(address));

    // Our row showing up proves the connection, the gate's consent and a primed
    // roster at once. Repeating the SET until then is harmless.
    let started = Instant::now();
    while !sessions.get().iter().any(|s| s.id == id) {
        if started.elapsed() > CONNECT_TIMEOUT {
            bail!("couldn't register with the daemon within {CONNECT_TIMEOUT:?}; nothing was sent");
        }
        let _ = crate::emit_session_set(&client, &session);
        tokio::time::sleep(CONNECT_POLL).await;
    }

    let result = send_all(
        &client,
        &sessions.get(),
        &nicknames.get(),
        &target,
        &id,
        &machine,
        &body,
    )
    .await;
    let _ = crate::emit_session_del(&client, &session);
    tokio::time::sleep(CONNECT_POLL).await;
    result
}

async fn send_all(
    client: &MykoClient,
    sessions: &[Arc<Session>],
    nicknames: &[Arc<SessionNickname>],
    target: &Target,
    from: &SessionId,
    machine: &str,
    body: &str,
) -> Result<()> {
    let targets = resolve_targets(sessions, nicknames, target).map_err(anyhow::Error::msg)?;
    if targets.is_empty() {
        bail!("no active sessions on the roster; nothing was sent");
    }
    let body = labelled(machine, body);
    let mut failed = 0;
    for session in &targets {
        let cell = client.send_command::<SendMessage, SendMessageResult>(&SendMessage {
            to_session_id: session.id.clone(),
            body: body.clone(),
            as_session: Some(from.clone()),
            idempotency_key: None,
        });
        let result = crate::tools::await_command(cell, SEND_TIMEOUT).await;
        let name = handle(nicknames, session);
        let team = session.project.as_deref().unwrap_or("no team");
        let host = session
            .host
            .as_ref()
            .map_or("unknown host", |h| h.name.as_str());
        match result {
            Ok(sent) => println!("{name} ({team}, {host}): {}", outcome(&sent)),
            Err(e) => {
                failed += 1;
                println!("{name} ({team}, {host}): NOT SENT: {e}");
            }
        }
    }
    println!("--- {} sent, {failed} not sent", targets.len() - failed);
    if failed > 0 {
        bail!("{failed} of {} messages weren't sent", targets.len());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(id: &str, project: Option<&str>, kind: Option<&str>) -> Arc<Session> {
        Arc::new(Session {
            id: SessionId(Arc::from(id)),
            client_id: None,
            pid: 0,
            cwd: String::new(),
            git_branch: None,
            current_task: None,
            session_name: None,
            activity: None,
            kind: kind.map(str::to_string),
            connected_at: 0,
            last_activity_at: None,
            last_tool: None,
            last_tool_at: None,
            operator: None,
            host: None,
            project: project.map(str::to_string),
            channels_enabled: None,
        })
    }

    fn roster() -> Vec<Arc<Session>> {
        vec![
            session("aaaaaaaa-1", Some("muster"), None),
            session("aaaaaaaa-2", Some("muster"), None),
            session("bbbbbbbb-1", Some("catapp"), None),
            session("cccccccc-1", None, Some(TERMINAL_KIND)),
        ]
    }

    fn ids(found: Result<Vec<Arc<Session>>, String>) -> Vec<String> {
        found.unwrap().iter().map(|s| s.id.0.to_string()).collect()
    }

    fn named(token: &str) -> Target {
        Target::Named(token.to_string())
    }

    #[test]
    fn all_reaches_every_agent_but_no_terminal() {
        assert_eq!(ids(resolve_targets(&roster(), &[], &Target::All)).len(), 3);
    }

    #[test]
    fn a_team_name_reaches_each_of_its_sessions() {
        assert_eq!(
            ids(resolve_targets(&roster(), &[], &named("MUSTER"))),
            ["aaaaaaaa-1", "aaaaaaaa-2"]
        );
    }

    #[test]
    fn a_full_id_or_unique_prefix_reaches_one_session() {
        assert_eq!(
            ids(resolve_targets(&roster(), &[], &named("aaaaaaaa-2"))),
            ["aaaaaaaa-2"]
        );
        assert_eq!(
            ids(resolve_targets(&roster(), &[], &named("bbbbbbbb"))),
            ["bbbbbbbb-1"]
        );
    }

    #[test]
    fn unknown_ambiguous_and_short_names_are_refused() {
        for token in ["nobody", "aaaaaaaa", "aaaa"] {
            assert!(
                resolve_targets(&roster(), &[], &named(token)).is_err(),
                "{token}"
            );
        }
    }

    #[test]
    fn a_name_that_is_both_team_and_session_is_refused() {
        let mut sessions = roster();
        sessions.push(session("catapp", Some("other"), None));
        assert!(resolve_targets(&sessions, &[], &named("catapp")).is_err());
    }

    #[test]
    fn a_terminal_cant_be_addressed() {
        assert!(resolve_targets(&roster(), &[], &named("cccccccc-1")).is_err());
    }
}
