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
// REPLY so the plugin's own output can't be mistaken for it. That includes
// `hang_deliveries`, `hung` and `set_online` (MUS-81).

import { randomUUID } from "node:crypto";

import { $ } from "bun";

// The plugin's daemon connection goes through this, so the suite can cut it
// and restore it (`set_online`). Installed before the plugin loads.
let online = true;
const sockets = new Set<WebSocket>();
const RealWebSocket = globalThis.WebSocket;
class GatedWebSocket extends RealWebSocket {
  constructor(url: string | URL, protocols?: string | string[]) {
    // Offline, connect somewhere that refuses, so myko keeps retrying.
    super(online ? url : "ws://127.0.0.1:9", protocols);
    sockets.add(this);
    this.addEventListener("close", () => sockets.delete(this));
  }
}
globalThis.WebSocket = GatedWebSocket as typeof WebSocket;

const { MusterPlugin } = await import("../src/index.ts");

const REPLY = "@@conformance ";

// Idle: `session.prompt` starts a turn (the wake). Busy: a turn is running,
// so opencode queues the prompt behind it, and the model sees it next.
let idle = false;
let wokeWith: string | null = null;
let queued: string[] = [];
let loseDeliveries = false;
let lost = 0;
// A hung handover: session.prompt never resolves until the suite lets it go.
let hangDeliveries = false;
let hung = 0;
const hungResolvers: Array<() => void> = [];

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
      if (hangDeliveries) {
        // opencode hangs taking it. When it's let go, it takes it after all.
        hung++;
        await new Promise<void>((resolve) => hungResolvers.push(resolve));
        queued.push(text);
        return {};
      }
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
      if (wokeWith === null) return { text: null };
      // The plugin's session.prompt is a user turn, so opencode runs
      // chat.message for it, where muster-opencode drains the inbox. The model
      // sees both the prompt and whatever that drain adds.
      const drained = await userTurn();
      const text = [wokeWith, drained].filter((t) => t).join("\n");
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
    case "hang_deliveries":
      hangDeliveries = Boolean(cmd.on);
      if (!hangDeliveries) for (const resolve of hungResolvers.splice(0)) resolve();
      return { hung };
    case "hung":
      return { hung };
    case "set_online": {
      online = Boolean(cmd.on);
      if (!online) for (const ws of sockets) ws.close();
      const isOpen = () => [...sockets].some((ws) => ws.readyState === WebSocket.OPEN);
      const deadline = Date.now() + 10_000;
      while (isOpen() !== online && Date.now() < deadline) await sleep(50);
      if (online) await sleep(500);
      return { open: isOpen() };
    }
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
