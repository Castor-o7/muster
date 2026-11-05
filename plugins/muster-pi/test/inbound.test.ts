import { describe, expect, test } from "bun:test";

import type { NotifyChannelMeta } from "../src/entities.ts";
import { livePushText } from "../src/inbound.ts";

// Regression: the daemon clips a live push's `body` to CONTEXT_BODY_MAX_CHARS
// (2000) and says so with `body_truncated`. The full body stays in the durable
// message. Unless the injected text says it was clipped, the recipient reads
// the preview as the whole message, and once the push is acked the unread
// inbox won't bring it back.
describe("live push text", () => {
  const cases: [string, NotifyChannelMeta, string | undefined][] = [
    ["a complete body is injected as is", { body: "hello" }, "new message from alice [response_required=false]: hello"],
    ["a body that requires a response says so", { body: "hello", response_required: true }, "new message from alice [response_required=true]: hello"],
    ["an explicit false reply requirement is preserved", { body: "hello", response_required: false }, "new message from alice [response_required=false]: hello"],
    ["body_truncated false changes nothing", { body: "hello", body_truncated: false }, "new message from alice [response_required=false]: hello"],
    [
      "a truncated body says so and names the message",
      { body: "the first part", body_truncated: true, message_id: "msg_1" },
      "new message from alice [response_required=false]: the first part\n[preview truncated; read the full message with muster_messages (message msg_1)]",
    ],
    [
      "a truncated body without an id still says so",
      { body: "the first part", body_truncated: true },
      "new message from alice [response_required=false]: the first part\n[preview truncated; read the full message with muster_messages]",
    ],
    ["no body leaves it to the inbox drain", { body_truncated: true, message_id: "msg_1" }, undefined],
  ];

  for (const [name, meta, expected] of cases) {
    test(name, () => {
      expect(livePushText(meta, "alice")).toBe(expected);
    });
  }
});
