// A fake Prime Agent host for Muster's delivery conformance suite (MUS-69,
// crates/shim/tests/conformance.rs). It loads the real muster-prime-agent
// extension with a stand-in for the pi ExtensionAPI Prime Agent inherits, and
// reports, over a line protocol, what the model would have been shown. Run
// with bun from the suite; it isn't a `bun test` file.
//
// Pinned to @earendil-works/pi-coding-agent 0.84.0 (package-lock.json). What
// it assumes about Prime Agent is listed in docs/delivery-conformance.md. It
// is muster-pi's driver with Prime Agent's session manager: the session id
// comes from getSessionId(), and getHeader() describes a top-level session
// (no rlmDepth), which the extension registers.
//
// Protocol: one JSON command per stdin line; one reply per command on stdout,
// prefixed with REPLY so the extension's own console.log lines can't be
// mistaken for it.
//   {"cmd":"start","session_file":path}  → {"session_id":id}
//   {"cmd":"arm_idle"}                   → {}
//   {"cmd":"woken_yet"}                  → {"woken":bool}
//   {"cmd":"wait_for_wake","ms":n}       → {"text":string|null}
//   {"cmd":"busy_boundary"}              → {"text":string|null}
//   {"cmd":"lose_deliveries","on":bool}  → {"lost":n}
//   {"cmd":"lost"}                       → {"lost":n}

import { randomUUID } from "node:crypto";

import extension from "../src/index.ts";

const REPLY = "@@conformance ";

type Handler = (event: unknown, ctx: unknown) => unknown;
const handlers = new Map<string, Handler[]>();

// Idle: a triggerTurn message starts a turn (the wake). Busy: a turn is
// running, so a message is steered into it.
let idle = false;
let wokeWith: string | null = null;
let steered: string[] = [];
let loseDeliveries = false;
let lost = 0;

const pi = {
  on(name: string, handler: Handler) {
    const list = handlers.get(name) ?? [];
    list.push(handler);
    handlers.set(name, list);
  },
  registerTool(_tool: unknown) {},
  registerCommand(_name: string, _command: unknown) {},
  async sendMessage(
    message: { content?: unknown },
    options?: { triggerTurn?: boolean; deliverAs?: string },
  ) {
    if (loseDeliveries) {
      // The extension handed it to pi, and pi lost it (it threw), after the
      // daemon had already handed it over.
      lost++;
      throw new Error("conformance: delivery lost");
    }
    const text = String(message.content ?? "");
    if (idle && options?.triggerTurn) {
      idle = false;
      wokeWith = text;
    } else {
      steered.push(text);
    }
  },
};

const ui = {
  notify(_m: string, _v?: string) {},
  setStatus(_k: string, _v: string) {},
  setWidget(_k: string, _lines: string[], _options?: unknown) {},
};
let sessionFile = "";
let sessionId = "";
const ctx = () => ({
  hasUI: false,
  ui,
  sessionManager: {
    getSessionFile: () => sessionFile,
    getSessionId: () => sessionId,
    // A top-level conversation: no rlmDepth, so it isn't an RLM sub-agent.
    getHeader: () => ({}),
  },
});

async function fire(name: string, event: unknown): Promise<unknown[]> {
  const results = [];
  for (const handler of handlers.get(name) ?? []) results.push(await handler(event, ctx()));
  return results;
}

/** A turn starts: pi runs before_agent_start, which drains the inbox. */
async function startTurn(prompt: string): Promise<string | null> {
  const results = (await fire("before_agent_start", { prompt, systemPrompt: "" })) as Array<
    { message?: { content?: string } } | undefined
  >;
  const injected = results.map((r) => r?.message?.content).filter((c): c is string => !!c);
  return injected.length ? injected.join("\n") : null;
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

async function handle(cmd: Record<string, unknown>): Promise<unknown> {
  switch (cmd.cmd) {
    case "start": {
      sessionFile = String(cmd.session_file);
      sessionId = randomUUID();
      await fire("session_start", { type: "session_start" });
      return { session_id: sessionId };
    }
    case "arm_idle":
      await fire("agent_settled", { type: "agent_settled" });
      idle = true;
      wokeWith = null;
      return {};
    case "woken_yet":
      return { woken: wokeWith !== null };
    case "wait_for_wake": {
      const deadline = Date.now() + Number(cmd.ms ?? 0);
      while (wokeWith === null && Date.now() < deadline) await sleep(50);
      if (wokeWith === null) return { text: null };
      // The woken turn starts: before_agent_start drains what else is unread.
      const drained = await startTurn("(woken by muster)");
      const text = [wokeWith, drained].filter((t) => t).join("\n");
      wokeWith = null;
      return { text };
    }
    case "busy_boundary": {
      idle = false;
      // A busy session's next delivery point: what was steered into the turn
      // since the last one, and the inbox drained when the next turn starts.
      await sleep(300);
      const drained = await startTurn("continue");
      const parts = [...steered, drained].filter((t): t is string => !!t);
      steered = [];
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

await extension(pi as never);
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
