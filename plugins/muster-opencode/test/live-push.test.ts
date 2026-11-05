import { describe, expect, test } from "bun:test";

import type { NotifyChannelMeta } from "../src/entities.js";
import { InboxPuller } from "../src/inbox-pull.js";
import { deliverLivePush, livePushText, type LivePushTarget } from "../src/live-push.js";

// A session: the daemon's unread set (a pushed message stays unread until
// acked, as for a session with acksPushes), opencode's prompts, and the puller
// that holds and acks.
function fakeSession(unread: string[], opts: { injectFails?: boolean } = {}) {
  const inbox = new Set(unread);
  const injected: string[] = [];
  const acked: string[] = [];
  const puller = new InboxPuller<{ messageId: string }>({
    read: async () => [...inbox].map((messageId) => ({ messageId })),
    ack: async (_s, ids) => {
      for (const id of ids) {
        inbox.delete(id);
        acked.push(id);
      }
    },
    render: (messages) => messages.map((m) => m.messageId).join(","),
  });
  const target: LivePushTarget = {
    inject: async (content) => {
      if (opts.injectFails) throw new Error("opencode lost it");
      injected.push(content);
    },
    hold: (id) => puller.holdPush("s", id),
    pull: () => puller.pull("s"),
  };
  return { target, injected, acked, inbox, puller };
}

const push = (id: string, body = `body of ${id}`): NotifyChannelMeta => ({ message_id: id, body });

describe("live push text", () => {
  test("a complete body is injected as is", () => {
    expect(livePushText({ body: "hello" }, "alice")).toBe("new message from alice [response_required=false]: hello");
  });

  test("a truncated body says so and names the message", () => {
    expect(livePushText({ body: "the first part", body_truncated: true, message_id: "m1" }, "alice")).toBe(
      "new message from alice [response_required=false]: the first part\n[preview truncated; read the full message with muster_messages (message m1)]",
    );
  });

  test("no body leaves it to the inbox", () => {
    expect(livePushText({ message_id: "m1" }, "alice")).toBeUndefined();
  });
});

describe("deliverLivePush", () => {
  test("acks the pushed message once opencode has taken it", async () => {
    const session = fakeSession(["m1"]);
    await deliverLivePush(push("m1", "hello"), "alice", session.target);
    expect(session.injected).toEqual(["new message from alice [response_required=false]: hello"]);
    expect(session.acked).toEqual(["m1"]);
  });

  // MUS-81: the message opencode failed to take stays unread for the next turn.
  test("acks nothing when opencode fails to take it, and the next pull shows it", async () => {
    const session = fakeSession(["m1"], { injectFails: true });
    await expect(deliverLivePush(push("m1"), "alice", session.target)).rejects.toThrow("opencode lost it");
    expect(session.acked).toEqual([]);
    expect((await session.puller.pull("s"))?.text).toBe("m1");
  });

  // Upstream #133/#134: a drain after the push acked other unread mail
  // without showing it, such as a message sent while the session was offline.
  test("leaves another unread message for the next pull", async () => {
    const session = fakeSession(["waited", "m1"]);
    await deliverLivePush(push("m1"), "alice", session.target);
    expect(session.acked).toEqual(["m1"]);
    expect((await session.puller.pull("s"))?.text).toBe("waited");
  });

  test("a pull while opencode is still taking the push doesn't show it too", async () => {
    const session = fakeSession(["m1"]);
    let take!: () => void;
    const taking = new Promise<void>((resolve) => { take = resolve; });
    session.target.inject = async (content) => {
      await taking;
      session.injected.push(content);
    };
    const delivering = deliverLivePush(push("m1"), "alice", session.target);
    expect(await session.puller.pull("s")).toBeNull();
    take();
    await delivering;
    expect(session.acked).toEqual(["m1"]);
  });

  // The race: with acksPushes the message stays unread after its push, so a
  // turn-start pull can read it before the push lands. That turn shows it;
  // the push mustn't show it a second time.
  test("a push a pull already holds isn't shown again, and the pull acks it", async () => {
    const session = fakeSession(["m1"]);
    const pull = await session.puller.pull("s");
    expect(pull?.text).toBe("m1");
    await deliverLivePush(push("m1"), "alice", session.target);
    expect(session.injected).toEqual([]);
    expect(session.acked).toEqual([]);
    await pull!.commit();
    expect(session.acked).toEqual(["m1"]);
  });

  test("a push skipped for a pull that's then released is shown by the next pull", async () => {
    const session = fakeSession(["m1"]);
    const late = await session.puller.pull("s");
    await deliverLivePush(push("m1"), "alice", session.target);
    late!.release();
    expect(session.injected).toEqual([]);
    expect((await session.puller.pull("s"))?.text).toBe("m1");
  });

  test("a pull that starts while the push is held, then a push for it, shows it once", async () => {
    // Turn-start pull and push racing in the other order: the push holds
    // first, so the pull skips it and the push shows it.
    const session = fakeSession(["m1"]);
    let take!: () => void;
    const taking = new Promise<void>((resolve) => { take = resolve; });
    session.target.inject = async (content) => {
      await taking;
      session.injected.push(content);
    };
    const delivering = deliverLivePush(push("m1"), "alice", session.target);
    expect(await session.puller.pull("s")).toBeNull();
    take();
    await delivering;
    expect(session.injected.length).toBe(1);
    expect(session.acked).toEqual(["m1"]);
  });

  test("a push that names no message is injected and acks nothing", async () => {
    const session = fakeSession([]);
    await deliverLivePush({ body: "hello" }, "alice", session.target);
    expect(session.injected.length).toBe(1);
    expect(session.acked).toEqual([]);
  });

  test("a push with no body shows the inbox and acks it once shown", async () => {
    const session = fakeSession(["m1"]);
    await deliverLivePush({ message_id: "m1" }, "alice", session.target);
    expect(session.injected).toEqual(["m1"]);
    expect(session.acked).toEqual(["m1"]);
  });

  test("a push with no body acks nothing when opencode fails to take the inbox", async () => {
    const session = fakeSession(["m1"], { injectFails: true });
    await expect(deliverLivePush({}, "alice", session.target)).rejects.toThrow("opencode lost it");
    expect(session.acked).toEqual([]);
    expect((await session.puller.pull("s"))?.text).toBe("m1");
  });
});
