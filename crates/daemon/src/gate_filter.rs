//! What a paired machine may send over the gate's WebSocket.
//!
//! myko applies a client's events directly and trusts a command's `asSession`,
//! and it has no authorization hook of its own. So the gate reads each frame
//! from a paired machine and passes only what its shim needs, for sessions that
//! machine owns. Everything else, including anything it can't parse, is refused.

use std::path::Path;

use myko::wire::MykoMessage;
use serde_json::Value;
use tokio_tungstenite::tungstenite::Message;

use crate::credentials;

/// Queries the shim subscribes to. Messages aren't here: reading them goes
/// through `ReadMessages`, which is checked against the caller's session.
const QUERIES: &[&str] = &[
    "GetAllSessions",
    "GetAllRooms",
    "GetAllRoomMembers",
    "GetAllSessionNicknames",
];

/// Commands that act as the connection's session, or as `asSession` if named.
const COMMANDS_AS_CALLER: &[&str] = &[
    "SendMessage",
    "BroadcastMessage",
    "ReadMessages",
    "AckMessages",
    "JoinRoom",
    "LeaveRoom",
];

/// Setters on the session named by the command's `id`.
const COMMANDS_ON_SESSION: &[&str] = &[
    "SetSessionActivity",
    "SetSessionCurrentTask",
    "SetSessionGitBranch",
    "SetSessionLastActivityAt",
    "SetSessionLastTool",
    "SetSessionLastToolAt",
    "SetSessionSessionName",
];

/// Why a frame was refused. Holds names only, never message content.
#[derive(Debug, PartialEq, Eq)]
pub struct Refused(pub String);

/// Decide on one client frame, returning the frame to forward. Control frames
/// are left to the WebSocket layer. A frame carrying a `Session` event comes back
/// with the machine's verified host and the daemon's operator written into it.
pub fn check_frame(
    state_dir: &Path,
    machine: &str,
    operator: Option<&str>,
    frame: Message,
) -> Result<Message, Refused> {
    let mut message = match &frame {
        Message::Text(text) => serde_json::from_str::<MykoMessage>(text)
            .map_err(|_| Refused("unparseable text frame".into()))?,
        Message::Binary(data) => ciborium::de::from_reader::<MykoMessage, _>(data.as_ref())
            .map_err(|_| Refused("unparseable binary frame".into()))?,
        _ => return Ok(frame),
    };
    if carries_nul(&message) {
        return Err(Refused("NUL in a key or value".into()));
    }
    check_message(state_dir, machine, &message)?;

    let events: &mut [myko::wire::MEvent] = match &mut message {
        MykoMessage::Event(event) => std::slice::from_mut(event),
        MykoMessage::EventBatch(events) => events,
        _ => return Ok(frame),
    };
    for event in events {
        attest_session(&mut event.item, machine, operator);
    }
    match frame {
        Message::Text(_) => serde_json::to_string(&message)
            .map(Message::text)
            .map_err(|_| Refused("couldn't re-encode a Session event".into())),
        _ => {
            let mut bytes = Vec::new();
            ciborium::ser::into_writer(&message, &mut bytes)
                .map_err(|_| Refused("couldn't re-encode a Session event".into()))?;
            Ok(Message::binary(bytes))
        }
    }
}

/// myko strips NUL from an event's keys and values after this filter has run, so
/// `"id\\u0000"` would become `id` and replace the one that was checked. Nothing
/// legitimate contains NUL, so refuse it everywhere rather than try to match.
fn carries_nul(message: &MykoMessage) -> bool {
    serde_json::to_value(message).map_or(true, |v| value_has_nul(&v))
}

fn value_has_nul(value: &Value) -> bool {
    match value {
        Value::String(s) => s.contains('\0'),
        Value::Array(items) => items.iter().any(value_has_nul),
        Value::Object(map) => map
            .iter()
            .any(|(k, v)| k.contains('\0') || value_has_nul(v)),
        _ => false,
    }
}

/// Replace what a machine claims about itself on its `Session` row. Its `os` and
/// `arch` stay as claimed: they're only informational.
fn attest_session(item: &mut Value, machine: &str, operator: Option<&str>) {
    let Some(item) = item.as_object_mut() else {
        return;
    };
    if let Some(operator) = operator {
        item.insert("operator".into(), Value::from(operator));
    }
    let host = item
        .entry("host")
        .or_insert_with(|| serde_json::json!({"name": "", "os": "", "arch": ""}));
    // A claim of `null` or anything else that isn't a host is replaced too.
    if !host.is_object() {
        *host = serde_json::json!({"name": "", "os": "", "arch": ""});
    }
    if let Some(host) = host.as_object_mut() {
        host.insert("name".into(), Value::from(machine));
    }
}

fn check_message(state_dir: &Path, machine: &str, message: &MykoMessage) -> Result<(), Refused> {
    match message {
        MykoMessage::Ping(_) | MykoMessage::QueryCancel(_) | MykoMessage::QueryWindow(_) => Ok(()),
        MykoMessage::Query(query) => {
            if QUERIES.contains(&query.query_id.as_ref()) {
                Ok(())
            } else {
                Err(Refused(format!("query {}", query.query_id)))
            }
        }
        MykoMessage::Command(wrapped) => {
            let id = wrapped.command_id.as_str();
            if COMMANDS_AS_CALLER.contains(&id) {
                match wrapped.command.get("asSession") {
                    None | Some(Value::Null) => Ok(()),
                    Some(named) => require_owned(state_dir, machine, named, id),
                }
            } else if COMMANDS_ON_SESSION.contains(&id) {
                let named = wrapped.command.get("id").unwrap_or(&Value::Null);
                require_owned(state_dir, machine, named, id)
            } else {
                Err(Refused(format!("command {id}")))
            }
        }
        MykoMessage::Event(event) => check_event(state_dir, machine, event),
        MykoMessage::EventBatch(events) => events
            .iter()
            .try_for_each(|event| check_event(state_dir, machine, event)),
        other => Err(Refused(format!("message {}", variant_name(other)))),
    }
}

/// The only event a shim sends is its own `Session` row.
fn check_event(state_dir: &Path, machine: &str, event: &myko::wire::MEvent) -> Result<(), Refused> {
    if event.item_type.as_ref() != "Session" {
        return Err(Refused(format!("event on {}", event.item_type)));
    }
    let named = event.item.get("id").unwrap_or(&Value::Null);
    require_owned(state_dir, machine, named, "Session event")
}

fn require_owned(
    state_dir: &Path,
    machine: &str,
    session: &Value,
    what: &str,
) -> Result<(), Refused> {
    let Some(session) = session.as_str() else {
        return Err(Refused(format!("{what} without a session id")));
    };
    match credentials::claim_session(state_dir, machine, session) {
        Ok(true) => Ok(()),
        Ok(false) => Err(Refused(format!("{what} for session {session}"))),
        Err(e) => Err(Refused(format!("{what}: can't read session bindings: {e}"))),
    }
}

fn variant_name(message: &MykoMessage) -> &'static str {
    match message {
        MykoMessage::View(_) => "view",
        MykoMessage::Report(_) => "report",
        MykoMessage::Benchmark(_) => "benchmark",
        _ => "other",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn frame(value: Value) -> Message {
        Message::text(value.to_string())
    }

    fn command(id: &str, body: Value) -> Message {
        frame(json!({"event": "ws:m:command", "data": {"command": body, "commandId": id}}))
    }

    fn event(item_type: &str, id: &str) -> Message {
        frame(json!({"event": "ws:m:event", "data": {
            "item": {"id": id},
            "changeType": "SET",
            "itemType": item_type,
            "createdAt": "2026-10-30T00:00:00Z",
        }}))
    }

    fn query(id: &str) -> Message {
        frame(json!({"event": "ws:m:query", "data": {
            "query": {"tx": "t"},
            "queryId": id,
            "queryItemType": "x",
        }}))
    }

    fn check(state_dir: &Path, machine: &str, frame: Message) -> Result<Message, Refused> {
        check_frame(state_dir, machine, None, frame)
    }

    /// A directory where machine `a1` owns session `mine`.
    fn dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        assert!(credentials::claim_session(dir.path(), "a1", "mine").unwrap());
        dir
    }

    #[test]
    fn frames_decode_as_myko_expects() {
        // Guards the test helpers: if these stop parsing, every refusal below
        // would pass for the wrong reason.
        for f in [
            command("SendMessage", json!({"toSessionId": "x", "body": "hi"})),
            event("Session", "mine"),
            query("GetAllSessions"),
        ] {
            let Message::Text(text) = &f else {
                unreachable!()
            };
            serde_json::from_str::<MykoMessage>(text).expect("helper frame must parse");
        }
    }

    #[test]
    fn a_machine_can_act_for_its_own_session() {
        let dir = dir();
        let p = dir.path();
        assert!(check(p, "a1", event("Session", "mine")).is_ok());
        assert!(
            check(
                p,
                "a1",
                command("SendMessage", json!({"asSession": "mine"}))
            )
            .is_ok()
        );
        assert!(
            check(
                p,
                "a1",
                command("SetSessionActivity", json!({"id": "mine"}))
            )
            .is_ok()
        );
        assert!(check(p, "a1", query("GetAllSessions")).is_ok());
    }

    #[test]
    fn a_command_without_as_session_binds_to_the_connection() {
        let dir = dir();
        assert!(check(dir.path(), "murph", command("SendMessage", json!({}))).is_ok());
    }

    #[test]
    fn another_machines_session_is_refused() {
        let dir = dir();
        let p = dir.path();
        for f in [
            event("Session", "mine"),
            command("SendMessage", json!({"asSession": "mine"})),
            command("ReadMessages", json!({"asSession": "mine"})),
            command("AckMessages", json!({"asSession": "mine"})),
            command("SetSessionCurrentTask", json!({"id": "mine"})),
        ] {
            assert!(check(p, "murph", f.clone()).is_err(), "{f:?}");
        }
    }

    #[test]
    fn raw_writes_to_other_entities_are_refused() {
        let dir = dir();
        for item_type in ["Message", "MessageRead", "Room", "RoomMember", "Client"] {
            assert!(
                check(dir.path(), "a1", event(item_type, "mine")).is_err(),
                "{item_type}"
            );
        }
    }

    #[test]
    fn one_bad_event_refuses_a_whole_batch() {
        let dir = dir();
        let batch = frame(json!({"event": "ws:m:event-batch", "data": [
            {"item": {"id": "mine"}, "changeType": "SET", "itemType": "Session", "createdAt": "t"},
            {"item": {"id": "m"}, "changeType": "DEL", "itemType": "Message", "createdAt": "t"},
        ]}));
        assert!(check(dir.path(), "a1", batch).is_err());
    }

    #[test]
    fn unlisted_queries_and_commands_are_refused() {
        let dir = dir();
        let p = dir.path();
        assert!(check(p, "a1", query("GetAllMessages")).is_err());
        assert!(check(p, "a1", query("GetAllMessageReads")).is_err());
        assert!(check(p, "a1", command("SomethingNew", json!({}))).is_err());
    }

    #[test]
    fn unparseable_frames_are_refused() {
        let dir = dir();
        let p = dir.path();
        assert!(check(p, "a1", Message::text("not json")).is_err());
        assert!(check(p, "a1", Message::binary(vec![0xff, 0x00])).is_err());
    }

    fn forged_session() -> Value {
        json!({"event": "ws:m:event", "data": {
            "item": {
                "id": "mine",
                "operator": "forged@example.com",
                "host": {"name": "forged", "os": "macos", "arch": "aarch64"},
            },
            "changeType": "SET",
            "itemType": "Session",
            "createdAt": "t",
        }})
    }

    fn item_of(message: MykoMessage) -> Value {
        match message {
            MykoMessage::Event(event) => event.item,
            other => panic!("expected an event, got {other:?}"),
        }
    }

    #[test]
    fn a_session_event_cannot_choose_its_host_or_operator() {
        let dir = dir();
        let out = check_frame(
            dir.path(),
            "a1",
            Some("op@example.com"),
            frame(forged_session()),
        );
        let Message::Text(text) = out.unwrap() else {
            panic!("a text frame should stay text");
        };
        let item = item_of(serde_json::from_str(&text).unwrap());
        assert_eq!(item["operator"], "op@example.com");
        assert_eq!(item["host"]["name"], "a1");
        assert_eq!(item["host"]["os"], "macos");
    }

    #[test]
    fn nul_suffixed_keys_are_refused() {
        // Found by review: myko strips the NUL after the filter has run, so this
        // would have taken over `victim` while claiming only `mine`.
        let dir = dir();
        for change in ["SET", "DEL"] {
            let forged = json!({"event": "ws:m:event", "data": {
                "item": {"id": "mine", "id\u{0}": "victim", "pid": 1, "cwd": "/", "connectedAt": 1},
                "changeType": change,
                "itemType": "Session",
                "createdAt": "t",
            }});
            assert!(check(dir.path(), "a1", frame(forged.clone())).is_err());
            let mut bytes = Vec::new();
            ciborium::ser::into_writer(&forged, &mut bytes).unwrap();
            assert!(check(dir.path(), "a1", Message::binary(bytes)).is_err());
            let batch = json!({"event": "ws:m:event-batch", "data": [forged["data"]]});
            assert!(check(dir.path(), "a1", frame(batch)).is_err());
        }
    }

    #[test]
    fn nul_keys_cannot_override_attestation() {
        let dir = dir();
        let forged = json!({"event": "ws:m:event", "data": {
            "item": {
                "id": "mine",
                "operator\u{0}": "forged@example.com",
                "host": {"name": "a", "os": "o", "arch": "r", "name\u{0}": "forged"},
            },
            "changeType": "SET",
            "itemType": "Session",
            "createdAt": "t",
        }});
        assert!(check_frame(dir.path(), "a1", Some("op@example.com"), frame(forged)).is_err());
    }

    #[test]
    fn a_nul_in_a_command_is_refused() {
        let dir = dir();
        assert!(
            check(
                dir.path(),
                "a1",
                command(
                    "SendMessage",
                    json!({"asSession": "mine", "body": "a\u{0}b"})
                )
            )
            .is_err()
        );
    }

    #[test]
    fn a_null_host_is_still_attested() {
        let dir = dir();
        let forged = json!({"event": "ws:m:event", "data": {
            "item": {"id": "mine", "host": null},
            "changeType": "SET",
            "itemType": "Session",
            "createdAt": "t",
        }});
        let Message::Text(text) = check(dir.path(), "a1", frame(forged)).unwrap() else {
            panic!("a text frame should stay text");
        };
        assert_eq!(
            item_of(serde_json::from_str(&text).unwrap())["host"]["name"],
            "a1"
        );
    }

    #[test]
    fn a_cbor_session_event_is_attested_too() {
        let dir = dir();
        let mut bytes = Vec::new();
        ciborium::ser::into_writer(&forged_session(), &mut bytes).unwrap();
        let out = check_frame(
            dir.path(),
            "a1",
            Some("op@example.com"),
            Message::binary(bytes),
        );
        let Message::Binary(data) = out.unwrap() else {
            panic!("a binary frame should stay binary");
        };
        let item = item_of(ciborium::de::from_reader(data.as_ref()).unwrap());
        assert_eq!(item["operator"], "op@example.com");
        assert_eq!(item["host"]["name"], "a1");
    }

    #[test]
    fn without_a_daemon_operator_the_claim_stands() {
        let dir = dir();
        let Message::Text(text) =
            check_frame(dir.path(), "a1", None, frame(forged_session())).unwrap()
        else {
            panic!("a text frame should stay text");
        };
        let item = item_of(serde_json::from_str(&text).unwrap());
        assert_eq!(item["operator"], "forged@example.com");
        assert_eq!(item["host"]["name"], "a1");
    }

    #[test]
    fn a_cbor_frame_is_checked_the_same_way() {
        let dir = dir();
        let value = json!({"event": "ws:m:event", "data": {
            "item": {"id": "mine"},
            "changeType": "SET",
            "itemType": "Session",
            "createdAt": "t",
        }});
        let mut bytes = Vec::new();
        ciborium::ser::into_writer(&value, &mut bytes).unwrap();
        let f = Message::binary(bytes);
        assert!(check(dir.path(), "a1", f.clone()).is_ok());
        assert!(check(dir.path(), "murph", f).is_err());
    }
}
