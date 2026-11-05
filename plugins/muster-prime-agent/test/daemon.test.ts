import { describe, expect, test } from "bun:test";

import { MusterDaemon } from "../src/daemon.ts";

// MUS-90: a reconnect's drain and a live push can both get the same message.
describe("a message is shown once, by a push or a drain", () => {
  const make = () =>
    new MusterDaemon({
      address: "ws://127.0.0.1:1",
      cwd: "/tmp",
      identity: { operator: "test", host: { name: "test", os: "linux", arch: "x64" } } as any,
    });
  const message = (messageId: string, body: string) => ({ messageId, fromSessionId: "peer", body, responseRequired: false });

  test("claimShown says yes once per message and session", () => {
    const daemon = make();
    expect(daemon.claimShown("ses_a", "m1")).toBe(true);
    expect(daemon.claimShown("ses_a", "m1")).toBe(false);
    expect(daemon.claimShown("ses_b", "m1")).toBe(true);
  });

  test("a drain acks a message a push already showed, but doesn't show it again", async () => {
    const daemon = make();
    const acked: string[][] = [];
    (daemon as any).send = async (cmd: any) => {
      if (cmd?.commandId === "AckMessages") {
        acked.push([...cmd.command.messageIds]);
        return {};
      }
      return { messages: [message("pushed", "already pushed"), message("waited", "waited out the drop")] };
    };
    expect(daemon.claimShown("ses_a", "pushed")).toBe(true); // the live push showed it
    const text = await daemon.drainInbox("ses_a");
    expect(text).toContain("waited out the drop");
    expect(text).not.toContain("already pushed");
    expect(acked.flat().sort()).toEqual(["pushed", "waited"]);
    expect(daemon.claimShown("ses_a", "waited")).toBe(false);
  });

  test("a drain of only already-shown mail shows nothing", async () => {
    const daemon = make();
    (daemon as any).send = async (cmd: any) =>
      cmd?.commandId === "AckMessages" ? {} : { messages: [message("pushed", "already pushed")] };
    daemon.claimShown("ses_a", "pushed");
    expect(await daemon.drainInbox("ses_a")).toBeNull();
  });

  test("a session's shown ids go when it deregisters", () => {
    const daemon = make();
    daemon.registerSession("ses_gone");
    expect(daemon.claimShown("ses_gone", "m1")).toBe(true);
    expect((daemon as any).shown.has("ses_gone")).toBe(true);
    daemon.deregisterSession("ses_gone");
    expect((daemon as any).shown.has("ses_gone")).toBe(false);
  });
});
