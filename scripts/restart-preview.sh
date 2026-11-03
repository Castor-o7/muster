#!/bin/zsh
# Report what the next daemon restart will remove, without changing anything.
# At boot the daemon deletes every row whose parent is gone: room mail to a
# deleted room, read records of a deleted message, and memberships of reaped
# sessions or deleted rooms. Since MUS-46, direct mail to a reaped session is
# kept until the 14-day sweep, so it's reported as kept, not removed.
# Run this before approving a restart.
# Needs psql. Uses $MYKO_POSTGRES_URL, as the daemon does, or the first argument.
set -u
URL=${1:-${MYKO_POSTGRES_URL:-}}
if [[ -z $URL ]]; then
  print -u2 "usage: $0 [postgres-url]   (or set MYKO_POSTGRES_URL)"
  exit 2
fi

# Each query rebuilds the same state, because a read-only session can't
# create views.
STATE="with latest as (
  select distinct on (item_type, item_id) item_type, item_id, change_type,
         event->'item' as item, created_at
  from myko_events order by item_type, item_id, id desc
), live as (select * from latest where change_type = 'SET'),
gone_messages as (
  select m.item_id
  from live m
  where m.item_type = 'Message'
    and m.item->>'toRoomId' is not null and not exists
      (select 1 from live r where r.item_type = 'Room' and r.item_id = m.item->>'toRoomId')
), kept_mail as (
  select m.item_id, m.item->>'toSessionId' as to_session
  from live m
  where m.item_type = 'Message'
    and m.item->>'toSessionId' is not null and not exists
      (select 1 from live s where s.item_type = 'Session' and s.item_id = m.item->>'toSessionId')
    and m.item_id not in (select item_id from gone_messages)
), reads as (
  select item->>'messageId' as message_id, item->>'sessionId' as session_id
  from live where item_type = 'MessageRead'
)"

PGOPTIONS='-c default_transaction_read_only=on' exec psql "$URL" -X -q -v ON_ERROR_STOP=1 <<SQL
\echo 'Rows the next restart will remove (live now, parent gone):'
$STATE
select 'Message' as type, count(*) as rows from gone_messages
union all
select 'MessageRead', count(*) from live r
  where r.item_type = 'MessageRead'
    and (r.item->>'messageId' in (select item_id from gone_messages)
      or not exists (select 1 from live m where m.item_type = 'Message' and m.item_id = r.item->>'messageId'))
union all
select 'RoomMember', count(*) from live rm
  where rm.item_type = 'RoomMember'
    and (not exists (select 1 from live s where s.item_type = 'Session' and s.item_id = rm.item->>'sessionId')
      or not exists (select 1 from live r where r.item_type = 'Room' and r.item_id = rm.item->>'roomId'));

\echo 'Kept, not removed: direct mail to reaped sessions, under 14 days (MUS-46):'
$STATE
select count(*) as kept from kept_mail;

$STATE
select left(k.to_session, 8) as recipient,
       (select item->>'nickname' from latest n
         where n.item_type = 'SessionNickname' and n.item_id = k.to_session) as nickname,
       (select to_char(created_at, 'YYYY-MM-DD HH24:MI TZ') from latest s
         where s.item_type = 'Session' and s.item_id = k.to_session) as removed_at,
       count(*) as messages,
       count(*) filter (where not exists (select 1 from reads r
         where r.message_id = k.item_id and r.session_id = k.to_session)) as unread
from kept_mail k
group by k.to_session
order by messages desc;

\echo 'Not counted: empty auto-rooms, which the sweeper removes a few seconds after start, and'
\echo 'messages over 14 days old, which it prunes then too, as it does every few minutes.'
\echo 'Removed rows stay in the event log; nothing here is erased.'
SQL
