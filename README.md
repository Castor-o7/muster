# marshal

A coordination service that lets Claude Code, Codex, Pi, Prime Agent, and
opencode sessions—on one machine or spread across a network—see each other and
pass messages. After install, the `roster` resource shows every live session,
`send_message` reaches them, and harness-specific delivery surfaces inbound
peer messages.

## Install

Install the binaries, run the daemon, then wire the harnesses you use.

### 1. Install the binaries

```bash
cargo install marshal-shim marshal-daemon
# optional but recommended for visibility into the live roster
cargo install marshal-tui
```

This puts `marshal-shim` and `marshal-daemon` on your `PATH` (typically `~/.cargo/bin`).

### 2. Start the daemon

The daemon runs out-of-band — one daemon that every session's shim connects to (on the same machine, or reachable over the network), lifetime independent of any Claude Code session. Pick whichever fits:

```bash
# foreground in its own terminal
marshal-daemon

# or backgrounded under your shell job control
marshal-daemon &

# or under your favorite supervisor (systemd user unit, launchd, tmux pane, ...)
```

The daemon binds `127.0.0.1:6155` by default and writes its event log to `~/.local/state/marshal/events.jsonl`. Set `MARSHAL_BIND=0.0.0.0:6155` (or a LAN/mesh address) to let other hosts reach it, and consider TLS (below) before you do.

### 2a. TLS and pairing (optional)

The daemon runs in one of two modes.

**Plain (the default).** myko serves WebSocket and MCP directly on `MARSHAL_BIND`, and the hook listener takes requests without a token. There's no TLS, pairing, token or limit; the daemon is protected only by where it's bound, so keep it on loopback or a network you trust. The startup line says `TLS gate: off`.

| Port | Variable | Default | Serves |
|---|---|---|---|
| 6155 | `MARSHAL_BIND` | `127.0.0.1:6155` | myko's own WebSocket/MCP listener |
| 6156 | `MARSHAL_HOOK_BIND` | `127.0.0.1:6156` | The plain-HTTP hook listener; refuses browser origins |

**TLS (`marshal-daemon --tls`).** Everything below: the gate, pairing, tokens, the frame filter, the limits and the sender allowlist. The startup line says `TLS gate: on`. The daemon refuses to run plain while its state directory has paired machines, unless it's started with `--plain`, so a unit file that loses `--tls` can't quietly open a gated daemon. Given both `--tls` and `--plain`, it refuses to start. Turning TLS off leaves the certificate, tokens and pairings on disk, so turning it back on restores them.

| Port | Variable | Default | Serves |
|---|---|---|---|
| 6155 | `MARSHAL_BIND` | `127.0.0.1:6155` | **The gate**. TLS connections must present a per-machine access token; plain HTTP from the daemon's own host (loopback) must present the local token; plain HTTP from another machine gets `426` and is told to pair |
| 6156 | `MARSHAL_HOOK_BIND` | `127.0.0.1:6156` | The plain-HTTP hook listener; requires the local token, refuses browser origins |
| 6158 | `MARSHAL_MYKO_BIND` | `127.0.0.1:6158` | myko's own WebSocket/MCP listener, behind the gate; admits only what the gate forwards, refuses browser origins |

To accept sessions from other machines, start the daemon with `--tls`, set `MARSHAL_BIND=0.0.0.0:6155` (or a LAN/mesh address), and pair each machine (below). Tools running on the daemon's own host, like the shim and the TUI, connect to `ws://localhost:6155` in either mode. With TLS on they add the **local token**: the daemon writes it owner-only to `~/.config/marshal/local-token` (under `$XDG_CONFIG_HOME` if set) on first start, and the shim and TUI read it from there. A process that can't read that file gets `401`, and any request naming a browser `Origin` gets `403`, on the hook listener, the gate's local path, and myko's own listener. myko's listener admits only requests the gate forwarded, through a request filter in the patched myko-server under `vendor/myko-server`. Loopback isn't the trust boundary: a reverse proxy on the host forwards from loopback too. The local token is. The web UI needs TLS off for now.

Set `MARSHAL_OPERATOR` on the daemon to the operator it serves. Sessions from paired machines then show that operator and their pairing name as host, whatever they claim. On the daemon's own host, a session's claim is taken as given.

**Limits, with TLS on.** Each session may send a burst of 20 messages, then 30 a minute; a machine's sessions share one allowance, so new sessions don't buy more. Past that, a send gets "too many messages from this session" and says when to retry. A broadcast counts as one. A message body may be up to 64 KiB. Connections are capped per listener, and a refused connection gets `503`:

| Variable | Default | Caps |
|---|---|---|
| `MARSHAL_MAX_CONN_PER_IP` | 128 | connections one IP holds on the gate, before and after the token check (past it, the connection is closed) |
| `MARSHAL_MAX_CONN_PER_MACHINE` | 64 | connections one paired machine holds on the gate |
| `MARSHAL_MAX_CONN_LOCAL` | 256 | connections on the gate's local path |
| `MARSHAL_MAX_CONN_HOOKS` | 256 | connections on the hook listener |

**The operator's controls.** Run these on the daemon's host. `pause` and `resume` work in both modes; the sender allowlist applies only with TLS on, since without pairing there are no machine names to check. The running daemon applies a change within 2 seconds.

```bash
marshal-daemon pause              # no message wakes any session; mail is kept, and seen when someone types
marshal-daemon resume
marshal-daemon allow host-a       # from now on only listed machines may send or broadcast
marshal-daemon disallow host-a
marshal-daemon senders            # show the list; `senders --clear` lets any machine send again
```

The daemon's own host is always allowed, so an allowlist never cuts off the sessions running beside it. Machine names are compared without regard to case. The daemon names its own host the way the shim does, from `MARSHAL_HOST` or the hostname up to its first dot, and `senders` prints that name, so you can check it matches what the roster shows for sessions there. A session with no machine on its row is refused while a list is set. While wake-ups are paused, `marshal://roster` and `marshal-shim roster` report `"wakes_paused": true`. These files live in the state directory, so any process on the operator's account can change them.

Pairing codes and bearer tokens pasted into a message body are masked before the message is stored, and never reach a log line.

The daemon keeps its TLS certificate and the paired machines' token hashes in `MARSHAL_STATE_DIR` (default `~/.local/state/marshal`). See [SECURITY.md](SECURITY.md) and [docs/threat-model.md](docs/threat-model.md) for what the gate does and doesn't protect against.

#### Pair each machine

Pairing needs TLS on. Every machine other than the daemon's own host needs a pairing. (On the daemon's host, an unpaired shim connects to `ws://localhost:6155` with the local token, so it works without one. Pair it too if other people can log in to that machine.) On the daemon's host:

```bash
marshal-daemon pair --name host-a --address 192.0.2.10:6155
```

It prints a `marshal-shim pair marshal-pair:v2:…` command. Run that on the machine you're pairing, within 15 minutes. The code holds the gate's address, the certificate fingerprint to pin, and a one-time invite. `marshal-shim pair` exchanges the invite at the gate for that machine's access token and stores the token owner-only in `~/.config/marshal/pairing`. The code then stops working, and it also stops working once it expires, so a code that ends up in a transcript or a log can't be used later. Then restart that machine's Claude Code sessions.

- `--address` is how the paired machine should reach the gate. It can be left out when `MARSHAL_BIND` is a specific address rather than `0.0.0.0`.
- `--expires 30s`, `15m` or `2h` sets how long the code works. The default is 15 minutes. A used or expired code gets "the daemon refused this code"; ask for a new one.
- `marshal-shim pairing` shows a machine's pairing with its token masked.
- `marshal-shim pair` also accepts a `marshal-pair:v1:` code, which holds the token itself; pair again to retire one that has been seen.
- `marshal-daemon pairs` lists paired machines, and `marshal-daemon unpair host-a` revokes one. Its open connections close within seconds and new ones are refused; nothing needs restarting.
- `marshal-shim unpair` removes a machine's own pairing.
- Pairing again under the same name replaces that machine's token, so that machine's running sessions lose their connection until they restart. This is also how to rotate a machine's token.
- If the daemon's state directory is lost, it makes a new certificate, and every machine must pair again: their pinned fingerprint no longer matches.

A paired shim never sends anything unencrypted. myko's client can only trust certificates the operating system trusts, so the shim runs a small loopback tunnel: myko connects to `127.0.0.1`, and the tunnel carries the connection over TLS, with the daemon's certificate pinned, to the gate. The hooks make the same pinned TLS connection themselves. Codex, Pi, opencode and Prime Agent clients don't support pairing yet.

`scripts/test-gate-e2e.sh` checks all of this end to end with the real binaries (it needs zsh, curl, nc and python3).

### 3. Add the plugin in Claude Code

```text
/plugin marketplace add ignition-is-go/marshal
/plugin install marshal-shim@marshal
```

Restart the session. From here on, every Claude Code instance pointed at the daemon sees the marshal MCP server and can talk to its peers.

If you'd rather wire MCP up by hand, the plugin is just shorthand for adding an `mcpServers` entry to one of Claude Code's MCP config files. Pick the scope that fits:

**Project scope** — write a `.mcp.json` at the repo root (commit it to share with collaborators):

```json
{
  "mcpServers": {
    "marshal": { "command": "marshal-shim" }
  }
}
```

**User scope** — applies to every Claude Code session for your user. Add a top-level `mcpServers` entry to `~/.claude.json`:

```json
{
  "mcpServers": {
    "marshal": { "command": "marshal-shim" }
  }
}
```

Or let the CLI do it for you:

```bash
claude mcp add -s user marshal marshal-shim
```

> Note: `mcpServers` is **not** a supported key in `~/.claude/settings.json` — adding it there silently does nothing. It must live in `.mcp.json` (project) or `~/.claude.json` (user).

### 4. Allow the channel notifications (research-preview workaround)

Marshal pushes peer messages into your transcript via Claude Code's experimental `notifications/claude/channel` capability. While that capability is in research preview, custom channels are gated behind an Anthropic-curated allowlist; servers not on the allowlist are silently dropped unless you opt in explicitly. Until marshal is on the official list, launch Claude Code with one of:

```bash
# If you installed via the plugin (recommended)
claude --dangerously-load-development-channels plugin:marshal-shim@marshal

# If you wired the MCP server directly via .mcp.json
claude --dangerously-load-development-channels server:marshal
```

Both forms key on the same name you set: `plugin:<plugin>@<marketplace>` for plugin installs, `server:<mcpServers-key>` for direct wiring. Set whichever fits as a shell alias to make it permanent (e.g. `alias claude='claude --dangerously-load-development-channels plugin:marshal-shim@marshal'`).

Without the flag, `roster` and `send_message` still work — peer messages just don't surface as live `<channel>` blocks in your transcript. With the flag, every peer's `send_message` arrives as an inline notification while you're working.

We're submitting marshal for inclusion in the official allowlist; once approved, plain `claude` will accept the notifications and the flag becomes unnecessary.

### Pi: native extension

After starting a reachable daemon, install the published
[`@agent-marshal/marshal-pi`](https://www.npmjs.com/package/@agent-marshal/marshal-pi)
package:

```bash
pi install npm:@agent-marshal/marshal-pi
```

Restart Pi. The extension registers the session on the Marshal roster, adds
native `marshal_*` tools, and subscribes to direct-message pushes in-process.
Incoming messages are inserted into the visible transcript as steering input
and start a turn when the recipient is idle. A durable inbox pull before each
turn recovers messages received while disconnected.

No MCP shim or launcher wrapper is required. To use a remote daemon:

```bash
export MARSHAL_DAEMON_ADDRESS=ws://<marshal-host>:6155
pi
```

Use `/marshal-status` in Pi to verify the connection. See
[`plugins/marshal-pi/README.md`](plugins/marshal-pi/README.md) for the complete
tool, configuration, and development reference.

### Prime Agent: live conversation bridge

Install the Prime Agent extension package after starting a reachable daemon:

```bash
prime-agent package install npm:@agent-marshal/marshal-prime-agent
```

Restart Prime Agent. The extension registers each top-level conversation on the
roster, adds native `marshal_*` tools, and subscribes to direct-message pushes
in-process. RLM sub-agents remain part of their parent's Marshal session instead of
appearing as separate roster entries.
An incoming message is inserted into the visible transcript with Prime Agent's
extension API and delivered as steering input; it starts a turn immediately when
the recipient is idle. Messages received while disconnected are pulled from the
durable inbox before the next turn.

This is a dedicated Prime Agent package, separate from `marshal-pi`; no MCP shim
or launcher wrapper is needed. To use a remote daemon:

```bash
export MARSHAL_DAEMON_ADDRESS=ws://<marshal-host>:6155
prime-agent
```

Use `/marshal-status` in Prime Agent to verify the connection. See
[`plugins/marshal-prime-agent/README.md`](plugins/marshal-prime-agent/README.md) for tools,
development loading, and delivery details.

### Codex: local setup without Ansible

These steps give a laptop, workstation, or manually managed VM the same
Marshal behavior as an infrastructure-managed host. You need a reachable
`marshal-daemon`: use the daemon from step 2 above, or substitute the address
of a shared daemon. Install the Codex CLI itself first and confirm that `codex`
is on `PATH`.

#### 1. Install and wire the shim

On Linux x86-64 or macOS, the release installer downloads the correct shim,
copies it to `~/.local/bin`, adds the Marshal MCP server and lifecycle hooks to
`~/.codex/config.toml`, seeds the coordination instructions in
`~/.codex/AGENTS.md`, and pre-trusts the hooks:

```bash
curl -fsSL \
  https://github.com/ignition-is-go/marshal/releases/latest/download/install-codex.sh \
  | sh -s -- --daemon ws://<marshal-host>:6155
```

On Windows x64, run this from PowerShell:

```powershell
$download = Join-Path $env:TEMP 'marshal-shim.exe'
Invoke-WebRequest `
  -Uri 'https://github.com/ignition-is-go/marshal/releases/latest/download/marshal-shim-x86_64-pc-windows-gnu.exe' `
  -OutFile $download
& $download codex-setup --daemon 'ws://<marshal-host>:6155'
```

Windows installs the stable copy under
`%LOCALAPPDATA%\marshal\bin` and adds that directory to the user `PATH`.
Restart the shell after setup. Both commands are idempotent; rerun them to
refresh the shim, hooks, or daemon address.

For a source build, install `marshal-shim` with Cargo and run the same setup
command:

```bash
cargo install marshal-shim
marshal-shim codex-setup --daemon ws://<marshal-host>:6155
```

A normal `codex` launch now gets the Marshal tools and pulls direct messages
at prompt and tool boundaries. For a message to start a turn while Codex is
idle, the interactive CLI must run through:

```bash
marshal-shim codex-run [CODEX_ARGS...]
```

On Linux and macOS this attaches the TUI to Codex's managed app-server. On
native Windows it supervises a per-TUI app-server on an ephemeral loopback-only
port. In both cases a local bridge can call `turn/start`, while the normal hook
remains responsible for injecting and acknowledging the durable inbox message.
The launcher waits for the bridge's lifecycle subscription before attaching
the TUI, so `thread/started` registers new and resumed sessions on the Marshal
roster before their first prompt. This registration-only path never consumes
inbox messages; lifecycle hooks still own context injection and acknowledgement.
After Codex or the user names the selected thread, the bridge prefixes the
title with its stable Marshal handle, for example
`[sunny-summit] Fix reconnect handling`. Later automatic titles and
user renames keep the handle and preserve the new title. The bridge does not
name an empty thread, because doing so would prevent Codex from generating its
normal title.
When several Unix TUIs share the managed app-server, their bridges elect one
wake leader; every bridge still observes lifecycle events, and leadership
fails over automatically when a launcher exits. Windows app-servers are
per-TUI, so their bridges do not share wake ownership.

#### 2. Make idle wake the default for interactive Codex

Codex does not currently expose the shared app-server choice as a
`config.toml` setting, so make `codex-run` the interactive shell launcher.
Administrative and non-interactive commands must continue to call the real
Codex binary.

For Bash, add the following to `~/.bashrc`. For Zsh, add it to `~/.zshrc` and
replace `type -P codex` with `whence -p codex`:

```bash
export MARSHAL_CODEX_BIN="$(type -P codex)"

codex() {
    local arg direct=0
    for arg in "$@"; do
        case "$arg" in
            exec|e|review|login|logout|mcp|plugin|mcp-server|app-server|remote-control|completion|update|doctor|sandbox|debug|apply|a|archive|delete|unarchive|cloud|exec-server|features|help|-h|--help|-V|--version|--remote|--remote=*)
                direct=1
                ;;
        esac
    done

    if [ "$direct" -eq 1 ]; then
        "$MARSHAL_CODEX_BIN" "$@"
    else
        "$HOME/.local/bin/marshal-shim" codex-run "$@"
    fi
}
```

For Windows PowerShell, add this to
`$PROFILE.CurrentUserAllHosts` after opening a fresh shell:

```powershell
$script:MarshalCodexBin = (Get-Command codex -CommandType Application).Source
$script:MarshalShimBin = (Get-Command marshal-shim -CommandType Application).Source

function global:codex {
    $directCommands = @(
        'exec', 'e', 'review', 'login', 'logout', 'mcp', 'plugin',
        'mcp-server', 'app-server', 'remote-control', 'completion', 'update',
        'doctor', 'sandbox', 'debug', 'apply', 'a', 'archive', 'delete',
        'unarchive', 'cloud', 'exec-server', 'features', 'help', '-h',
        '--help', '-V', '--version'
    )
    $direct = $false
    foreach ($argument in $args) {
        $value = [string]$argument
        if (
            $directCommands -contains $value -or
            $value -eq '--remote' -or
            $value.StartsWith('--remote=')
        ) {
            $direct = $true
            break
        }
    }

    if ($direct) {
        & $script:MarshalCodexBin @args
        return
    }

    $previous = $env:MARSHAL_CODEX_BIN
    try {
        $env:MARSHAL_CODEX_BIN = $script:MarshalCodexBin
        & $script:MarshalShimBin codex-run @args
    } finally {
        $env:MARSHAL_CODEX_BIN = $previous
    }
}
```

Start a new shell, then run plain `codex` or `codex resume`. An interactive
session should have both a `marshal-shim codex-run` launcher and a
`marshal-shim codex-bridge` process, and it should appear in the Marshal roster
as soon as the TUI opens—even before its first prompt. Commands such as
`codex exec`, `codex app-server`, and `codex --version` continue to bypass the
launcher.
See [`docs/codex-live-delivery.md`](docs/codex-live-delivery.md) for the
delivery and trust model.

## What you get

Reads are MCP **resources**; writes are MCP **tools**. Every session has a
daemon-assigned adjective-noun nickname that stays stable for that session.
Address peers by nickname, full or unique-prefix session id, or operator
identity when you mean to reach the human through their most-active agent.

**Resources** (`resources/read`):

| Resource | Contents |
|---|---|
| `marshal://whoami` | This session's `{ session_id, pid, cwd, operator, host }`. |
| `marshal://roster` | Every live session: cwd, git branch, current task, last activity, room membership. |
| `marshal://rooms` | Every room and its members. |
| `marshal://messages` | Message history. Query params: `inbox`, `sent`, `unread`, `room`, `from`, `to_session`, `since`, `limit`. |

**Tools** (`tools/call`):

| Tool | Effect |
|---|---|
| `send_message(to, body)` | Interrupt one peer by nickname, session id/prefix, or operator identity. |
| `broadcast(to_room, body)` | Ambient room update; an `@mention` also directly interrupts that peer. |
| `join_room(room)` / `leave_room(room)` | Create/join or leave an ad-hoc room. |
| `set_status(text)` | Update this session's free-form status (the `current_task` field on the roster). |
| `ack_messages(message_ids)` | Mark message ids read for this session. |

A peer's `send_message` is persisted first, then Marshal attempts a synchronous
live-channel push when the recipient supports one. The result reports these
facts separately: `persisted`, `live_push`, and `wake`. Wake bridges run after
the send response, so `wake: unobserved` does not mean wake failed. Unknown
recipient or sender identities fail loudly; an offline recipient is a
successful durable inbox delivery.

Direct messages start or enter a recipient turn, so they consume transcript
context. Batch related details and reserve direct delivery for action, a
blocker, or a needed reply. Use an unmentioned room broadcast for FYI/progress.
Automatic body previews are bounded (2,000 characters per message and an
8,000-character hook-batch budget); the complete durable message remains
available through `marshal://messages`. Codex wakes coalesce for 30 seconds so
a burst can join the active turn.

### From a terminal

Outside any session, on a machine that reaches the daemon:

```bash
marshal-shim all "Full agenda today, please send your status."
marshal-shim tell catapp "Please pause before the next migration."
```

`all` reaches every session on the roster, and `tell` takes a team name, nickname or session id. Each session gets a direct message, which wakes it if idle, and you get one line per session. An unknown or ambiguous name is refused. The exit status is non-zero if anything wasn't sent.

Recipients see `[from a terminal on <machine>; it can't receive replies]`. That names the machine, not the person: any process on your account can run this command, including a Claude session, so these messages carry no extra authority. A message from `all` or `tell` is never the operator's approval. It doesn't count as a direct OK for a daemon restart or anything else that needs one, because a session can send the same message.

`marshal-shim roster` prints the roster as JSON for other programs. It only reads: it registers no session and doesn't appear on the roster. Each session has `observed` facts from the daemon's own record (whether the row is bound to a connection, whether it is holding a `/hook/wait`, and until when) and `declared` ones its own software reports, which any client can set to anything: machine, operator, `last_activity_at`, `activity`, `current_task` and the rest. `wake` says whether a message would wake the session, as on `marshal://roster`: `waiting` (a wait is held), `channel` or `busy` (as the session declares), or `none`. `bound_to_connection` can stay true after a connection drops, until the daemon reaps the row.

## Architecture

```
Claude Code ── marshal-shim (stdio MCP) ──────┐
Codex ──────── marshal-shim hooks/bridge ─────┤
opencode ───── marshal-opencode plugin ───────┼── ws://localhost:6155
Pi ─────────── marshal-pi extension ──────────┤          │
Prime Agent ── marshal-prime-agent extension ─┘          └── marshal-daemon
                                                            (roster + event log)
```

- **`marshal-daemon`** owns the live roster and the event log under `~/.local/state/marshal/events.jsonl`. One daemon serves every session that points its shim at it (local or remote). Run it under your favorite supervisor (or just `marshal-daemon &` in a terminal).
- **`marshal-shim`** is the per-session stdio MCP server Claude Code spawns. It announces the session to the daemon on connect, watches for inbound messages, and forwards them onto stdout as channel notifications.
- **`marshal-pi`** is the in-process Pi extension that registers sessions, exposes native tools, and injects direct messages as steering input.
- **`marshal-prime-agent`** is the in-process Prime Agent extension that registers conversations, exposes tools, and injects live direct messages.
- **`marshal-opencode`** provides the equivalent in-process opencode integration.
- **`marshal-tui`** (optional) is a live ratatui dashboard of the roster + recent messages.

## Configuring the daemon address

Marshal clients default to `ws://localhost:6155`. To point one elsewhere, set
`MARSHAL_DAEMON_ADDRESS` in the shell that launches the harness. For Claude
Code, you can instead pin it in a per-project `.mcp.json`:

```json
{
  "mcpServers": {
    "marshal": {
      "env": { "MARSHAL_DAEMON_ADDRESS": "ws://10.0.0.5:6155" }
    }
  }
}
```

A paired machine finds the daemon from its pairing (step 2a), which takes precedence over all of the above.

The daemon's bind address is set with `MARSHAL_BIND` (default `127.0.0.1:6155`; set a LAN or mesh address, or `0.0.0.0:6155`, to accept other machines).

## Workspace layout

| Component | Description |
|---|---|
| [`marshal-entities`](crates/entities/) | Shared entity types and the `SendMessage` server command. |
| [`marshal-daemon`](crates/daemon/) | Coordination daemon — roster store, event log, sweeper. |
| [`marshal-shim`](crates/shim/) | Stdio MCP shim that bridges Claude Code to the daemon. |
| [`marshal-tui`](crates/tui/) | Live terminal dashboard. |
| [`marshal-ui`](crates/ui/) | Leptos operator dashboard (build target only — not published). |
| [`@agent-marshal/marshal-pi`](plugins/marshal-pi/) | Pi native session extension. |
| [`@agent-marshal/marshal-prime-agent`](plugins/marshal-prime-agent/) | Prime Agent live-conversation extension. |
| [`marshal-opencode`](plugins/marshal-opencode/) | opencode native plugin. |

Releases are driven by [`cargo-flux`](https://github.com/ignition-is-go/cargo-flux): conventional-commit `feat:` / `fix:` / breaking changes on `main` cut a stable release; the same on `dev` cuts a prerelease. See `flux.toml` and `.github/workflows/release.yml`.

## License

MIT OR Apache-2.0

That covers Marshal's own source code. Marshal depends on myko, myko-macros and autosocket
(and myko-server for `marshal-daemon`), which declare the AGPL-3.0-or-later licence. A copy of
myko-server with one patch is vendored under `vendor/myko-server` and keeps that licence. Anyone
who distributes or runs binaries built from Marshal should assess the licence of the combined
work. This isn't legal advice.
