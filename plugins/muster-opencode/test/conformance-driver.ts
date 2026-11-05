// A fake opencode host for Muster's delivery conformance suite (MUS-69,
// crates/shim/tests/conformance.rs). It loads the real muster-opencode plugin
// with a stand-in for opencode's plugin input (the SDK client, bun's `$`) and
// reports, over a line protocol, what the model would have been shown. Run
// with bun from the suite; it isn't a `bun test` file.
//
// Pinned to @opencode-ai/plugin and @opencode-ai/sdk 1.17.11
// (package-lock.json). What it assumes about opencode is listed in
// docs/delivery-conformance.md.
//
// The protocol is muster-pi's (see its conformance-driver.ts): one JSON
// command per stdin line, one reply per command on stdout, prefixed with
// REPLY so the plugin's own output can't be mistaken for it.

import { randomUUID } from "node:crypto";

import { $ } from "bun";

import { MusterPlugin } from "../src/index.ts";

const REPLY = "@@conformance ";

// Idle: `session.prompt` starts a turn (the wake). Busy: a turn is running,
// so opencode queues the prompt behind it, and the model sees it next.
let idle = false;
let wokeWith: string | null = null;
let queued: string[] = [];
let loseDeliveries = false;
let lost = 0;

const client = {
  app: { log: async (_: unknown) => ({}) },
  tui: { showToast: async (_: unknown) => ({}) },
  session: {
    async prompt(req: { path: { id: string }; body: { parts: Array<{ text?: string }> } }) {
      if (loseDeliveries) {
        // The plugin handed it to opencode, which failed to take it, after
        // the daemon had already handed it over.
        lost++;
        throw new Error("conformance: delivery lost");
      }
      const text = req.body.parts.map((p) => p.text ?? "").join("");
      if (idle) {
        idle = false;
        wokeWith = text;
      } else {
        queued.push(text);
      }
      return {};
    },
  },
};

type Hooks = Record<string, (...args: never[]) => unknown>;
let hooks: Hooks = {};
let sessionId = "";

/** A user turn starts: opencode runs the `chat.message` hook, which drains the
 *  inbox into a visible part. */
async function userTurn(): Promise<string | null> {
  const output = {
    message: { id: `msg_${randomUUID()}`, sessionID: sessionId },
    parts: [] as Array<{ text?: string }>,
  };
  const chatMessage = hooks["chat.message"] as
    | ((input: { sessionID: string }, output: unknown) => Promise<void>)
    | undefined;
  await chatMessage?.({ sessionID: sessionId }, output);
  const injected = output.parts.map((p) => p.text ?? "").filter((t) => t);
  return injected.length ? injected.join("\n") : null;
}

async function fireEvent(type: string, properties: Record<string, unknown>) {
  const event = hooks.event as ((input: { event: unknown }) => Promise<void>) | undefined;
  await event?.({ event: { type, properties } });
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

async function handle(cmd: Record<string, unknown>): Promise<unknown> {
  switch (cmd.cmd) {
    case "start": {
      sessionId = `ses_${randomUUID().replace(/-/g, "").slice(0, 26)}`;
      await fireEvent("session.created", { info: { id: sessionId } });
      return { session_id: sessionId };
    }
    case "arm_idle":
      await fireEvent("session.idle", { sessionID: sessionId });
      idle = true;
      wokeWith = null;
      return {};
    case "woken_yet":
      return { woken: wokeWith !== null };
    case "wait_for_wake": {
      const deadline = Date.now() + Number(cmd.ms ?? 0);
      while (wokeWith === null && Date.now() < deadline) await sleep(50);
      const text = wokeWith;
      wokeWith = null;
      return { text };
    }
    case "busy_boundary": {
      idle = false;
      // What was queued into the running turn since the last boundary, and
      // what the next user turn's chat.message drains.
      await sleep(300);
      const drained = await userTurn();
      const parts = [...queued, drained].filter((t): t is string => !!t);
      queued = [];
      return { text: parts.length ? parts.join("\n") : null };
    }
    case "lose_deliveries":
      loseDeliveries = Boolean(cmd.on);
      return { lost };
    case "lost":
      return { lost };
    default:
      return { error: `unknown command ${String(cmd.cmd)}` };
  }
}

hooks = (await MusterPlugin({
  client,
  $,
  directory: process.cwd(),
  worktree: process.cwd(),
} as never)) as Hooks;
for await (const line of console) {
  if (!line.trim()) continue;
  let reply: unknown;
  try {
    reply = await handle(JSON.parse(line));
  } catch (e) {
    reply = { error: String(e) };
  }
  process.stdout.write(REPLY + JSON.stringify(reply) + "\n");
}
