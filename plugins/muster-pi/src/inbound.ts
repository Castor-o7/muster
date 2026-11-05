// Text a live push injects into the session. Kept free of pi and daemon
// imports so it can be tested directly.

import type { NotifyChannelMeta } from "./entities.ts";

/** The steer message for a live push, or `undefined` when the push carries no
 *  body (older daemons), in which case the caller pulls the inbox instead.
 *
 *  The daemon caps `body` at `CONTEXT_BODY_MAX_CHARS` and says so with
 *  `body_truncated`. The full message stays in the durable store, so say the
 *  preview was clipped and name the message for `muster_messages`. */
export function livePushText(meta: NotifyChannelMeta, who: string): string | undefined {
  if (!meta.body) return undefined;
  const text = `new message from ${who} [response_required=${meta.response_required ?? false}]: ${meta.body}`;
  if (!meta.body_truncated) return text;
  const id = meta.message_id ? ` (message ${meta.message_id})` : "";
  return `${text}\n[preview truncated; read the full message with muster_messages${id}]`;
}
