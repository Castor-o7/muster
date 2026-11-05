import { describe, expect, test } from "bun:test";

import { MusterDaemon } from "../src/daemon.ts";

const make = () =>
  new MusterDaemon({
    address: "ws://127.0.0.1:1",
    cwd: "/tmp",
    identity: { operator: "test", host: { name: "test", os: "linux", arch: "x64" } } as any,
  });

// A daemon whose inbox reads `messages` and records acks, as if connected.
function withInbox(daemon: MusterDaemon, messages: () => unknown[]) {
  const acked: string[][] = [];
  (daemon as any).connected = true;
  (daemon as any).send = async (cmd: any) => {
    if (cmd?.commandId === "AckMessages") {
      acked.push([...cmd.command.messageIds]);
      return {};
    }
    return { messages: messages() };
  };
  return acked;
}

const message = (messageId: string, body: string) => ({ messageId, fromSessionId: "peer", body, responseRequired: false });

describe("inbox pull", () => {
  test("returns at once while Muster is disconnected", async () => {
    const daemon = make();
    const outcome = await Promise.race([
      daemon.pullInbox("ses_disconnected"),
      new Promise<"blocked">((resolve) => setTimeout(() => resolve("blocked"), 100)),
    ]);
    expect(outcome).toBeNull();
  });
});

// MUS-90: a reconnect's pull and a live push can both get the same message.
describe("a message is shown once, by a push or a pull", () => {
  test("claimShown says yes once per message and session", () => {
    const daemon = make();
    expect(daemon.claimShown("ses_a", "m1")).toBe(true);
    expect(daemon.claimShown("ses_a", "m1")).toBe(false);
    expect(daemon.claimShown("ses_b", "m1")).toBe(true);
  });

  test("a pull acks a message a push already showed, but doesn't show it again", async () => {
    const daemon = make();
    const acked = withInbox(daemon, () => [message("pushed", "already pushed"), message("waited", "waited out the drop")]);
    expect(daemon.claimShown("ses_a", "pushed")).toBe(true); // the live push showed it
    const pull = await daemon.pullInbox("ses_a");
    expect(pull?.text).toContain("waited out the drop");
    expect(pull?.text).not.toContain("already pushed");
    expect(acked.flat()).toEqual(["pushed"]); // the unshown one waits for the commit
    await pull!.commit();
    expect(acked.flat().sort()).toEqual(["pushed", "waited"]);
    expect(daemon.claimShown("ses_a", "waited")).toBe(false);
  });

  test("a pull of only already-shown mail shows nothing", async () => {
    const daemon = make();
    withInbox(daemon, () => [message("pushed", "already pushed")]);
    daemon.claimShown("ses_a", "pushed");
    expect(await daemon.pullInbox("ses_a")).toBeNull();
  });

  test("a push skips a message a pull holds to show", async () => {
    const daemon = make();
    withInbox(daemon, () => [message("m1", "hello")]);
    const pull = await daemon.pullInbox("ses_a");
    expect(pull?.text).toContain("hello");
    expect(daemon.claimShown("ses_a", "m1")).toBe(false);
    await pull!.commit();
    expect(daemon.claimShown("ses_a", "m1")).toBe(false);
  });

  // Upstream #136: a pull that isn't shown leaves its messages unread.
  test("a released pull neither acks nor claims its messages", async () => {
    const daemon = make();
    const acked = withInbox(daemon, () => [message("m1", "hello")]);
    const pull = await daemon.pullInbox("ses_a");
    pull!.release();
    expect(acked).toEqual([]);
    expect((await daemon.pullInbox("ses_a"))?.text).toContain("hello");
  });
});
