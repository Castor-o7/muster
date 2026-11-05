# Operator-verified approvals (MUS-39)

Status: design, for the operator to choose from. No code yet. The stories are in
[roadmap.md](roadmap.md), and the attackers are those in
[threat-model.md](threat-model.md).

## The problem

Every session runs as the operator's account. So a session can send "Josh says
restart" as easily as a relay can, read the local token and the daemon's state
directory, and edit any file the daemon reads. MUS-5, MUS-6 and MUS-7 bound what a
session can do through Muster. None of them lets a session prove that the operator
said something. So far a restart, a deploy or a role has needed the operator to type
in the acting session's own terminal. That has held up the Rally rollout on Murph more
than once.

The fix is an approval: a short signed statement naming one action and one target,
made by a step only the human can perform.

## What an approval is

A statement, and an SSH signature over it, as Rally's watchman designation already
does (RAL-51, Rally's `docs/designation.md`):

```text
muster-approval v1
action: restart-daemon
target: murph:muster@e9ffc08
nonce: 5q0d2kq8v1m3t7xw9c4b6n2a1e
issued: 1793907000000
expires: 1793907600000
approver: joshd@pdx.edu
```

- `action` comes from the fixed list below.
- `target` is exactly what the action applies to.
- `nonce` is 128 random bits, in base32.
- `issued` and `expires` are in ms since the epoch, and `expires` is at most 10
  minutes after `issued`.
- `approver` names the key in `allowed_signers`.

**The signed bytes** are exactly these seven lines, in this order, as `key: value`
with one space after the colon. Each line ends in LF, including the last, with no
CR, no trailing whitespace, no blank lines and no comments. A verifier rebuilds the
bytes from the parsed fields and refuses a statement that doesn't round-trip to the
same bytes, so two parsers can't read one statement two ways (the lesson from
MUS-5's filter bypasses).

The signature is `ssh-keygen -Y sign -n muster-approval` over those bytes, with the
armored `SSH SIGNATURE` block that follows the statement. The `muster-approval`
namespace means a Rally designation signature can't pass as an approval, and an
approval can't pass as a designation. It's all text, so it can travel in a Muster
message, a PR comment or a paste. Whoever acts on it checks four things, all of
which a test must show:

1. **The signature** verifies with `ssh-keygen -Y verify -n muster-approval -I
   <approver> -f /etc/muster/allowed_signers`, or the same check in Rust. The
   allowed-signers file is one the operator's account can't change (see "Where
   the keys live").
2. **The action and target** match exactly what is about to be done. No prefixes and
   no wildcards: `murph:muster@e9ffc08` doesn't approve `murph:muster@e9ffc09`.
3. **It hasn't expired**, allowing a minute for clock skew.
4. **It hasn't been used.** The verifier records the nonce before acting. See
   "Reuse" for where that record lives and what follows from it.

A relay that carries a valid approval is shown as **operator-verified** on the
roster and in the message view. Anything else, however it's worded, is shown as
the session's claim.

### Actions

Start with a fixed list, so that a target can't be bent into a different action:

| action | target | checked by |
|---|---|---|
| `restart-daemon` | `<host>:<repo>@<commit>` | the Lamplighter, before restarting |
| `deploy` | `<host>:<repo>@<commit>` | the Lamplighter or the rollout script |
| `assign-role` | `<role>:<session id>` | the daemon (MUS-51, below) |
| `settings-edit` | `<host>:<path>@<sha256 of new content>` | the session making the edit |
| `merge`, only if the operator says so (question 4) | `<owner>/<repo>#<pr>@<head sha>` | the watchman, before merging |

Adding an action is a code change, and so needs review. `merge` is listed only as
a proposal. Until the operator says otherwise, a merge stays on their direct word.

## The choice for the operator: how the human proves it's them

The signing step must need something a session can't supply. There are two
candidates. Both sign with an SSH key and are checked against the same
`allowed_signers` file, so there is one verifier. They can be used together, one
for each host.

### Option A: `sudo` on the daemon's host

`muster-approve` is a small binary installed root-owned. The signing key is an SSH
key, `/etc/muster/approval_key`, owned by root with mode 0600. The operator runs
`sudo muster-approve restart-daemon murph:muster@e9ffc08`. It prints the statement
and the signature, made with `ssh-keygen -Y sign -n muster-approval -f
/etc/muster/approval_key`, and the operator pastes or sends them.

- **For:** it works on Murph today, over SSH, with nothing new to buy. The key never
  leaves root.
- **Against:** sudo's credential cache is a hole. Right after the operator runs
  sudo, a session sharing that tty could run it too. Claude Code's Bash tool has no
  tty, so per-tty tickets mostly protect against this, but "mostly" isn't a
  guarantee. Fix: `Defaults!/usr/local/bin/muster-approve timestamp_timeout=0` in a
  sudoers drop-in, so every approval asks for the password. Also, any NOPASSWD rule
  for the operator's account defeats this option, so the installer checks
  `sudo -l` and refuses if one applies.
- **Threat model:** this holds against a compromised session and a compromised paired
  machine. It doesn't hold against root on the daemon's host, which is already out of
  scope. A keylogger in the operator's account can capture the sudo password, which
  is the same exposure as sudo itself.

### Option B: Touch ID on the operator's laptop, with Secretive

This is what Rally already uses for the watchman designation. The key is a Secure
Enclave key in [Secretive](https://github.com/maxgoedjen/secretive), with **Require
Authentication** on, so every signature needs Touch ID. The key is ECDSA P-256 and
can't be exported. In a shell whose `SSH_AUTH_SOCK` is Secretive's agent,
`muster-approve restart-daemon murph:muster@e9ffc08 --key ~/muster-approval.pub`
prints the statement, then signs it with `ssh-keygen -Y sign -n muster-approval`
through the agent.

Use **a new Secretive key named "Muster approval"**, not Rally's designation key,
so that a Touch ID prompt from each key means one thing only. The namespaces
already keep the signatures apart. Separate keys keep the operator's reading of
the prompt apart too.

- **For:** a process can't cache or replay it, because every signature needs a
  finger on the sensor. Josh already runs Secretive for Rally. There's no helper to
  build and no keychain entitlement to work out. It works away from Murph.
- **Against:** only the laptop can approve, so Murph alone can't. An agent can ask
  Secretive's agent for a signature, which brings up the prompt. Only the operator
  can answer it, but see the next section.
- **Threat model:** stronger than A against anything in the operator's account,
  keyloggers included. A stolen, unlocked laptop still needs the fingerprint.

A Swift and CryptoKit helper remains a fallback if Secretive won't do for some
reason, at the cost of building it and settling the entitlement question.

### The prompt doesn't show what's being signed

Neither Touch ID nor a sudo password prompt shows the statement. So
`muster-approve` always prints the full statement, all seven lines, before it asks
for the signature, and the operator reads it before touching or typing. **A session
can put a different statement in front of the operator.** It might run
`muster-approve` itself with its own action and target, and so raise a Touch ID
prompt the operator expects for something else. Or it might hand over a statement
to sign that differs from what it described in chat. The defences:

- **Sign only in a shell you started yourself**, never one an agent opened or ran
  the command in.
- **Read the printed `action` and `target`**, not the agent's description of them.
- **Treat a Touch ID prompt from the "Muster approval" key that you didn't start as
  an attack**, and refuse it.

Rally's designation doc carries the same warning.

### Recommendation

Use **B as the primary and A as the fallback**, each key listed in
`allowed_signers` under its own name. This is the operator's call. A alone is the
quickest route to unblocking restarts on Murph.

## Where the keys live

What matters is that **the verifier's keys, and the verifier itself, must be
writable only by root**. Otherwise a session swaps in its own key and signs
anything, or swaps the program that checks.

- Public keys: `/etc/muster/allowed_signers`, in `ssh-keygen`'s allowed-signers
  format, with each key restricted to `namespaces="muster-approval"`. It's owned by
  root with mode 0644, on every host that verifies. Installing or rotating a key
  needs sudo, once per host.
- Every verifier refuses to verify if that file, the binary doing the check, or any
  directory above either is writable by anyone but root. That's the same rule as
  Rally's `serve --secure`. The check runs each time, because one made at install
  time can go stale.
- Where the daemon is the verifier (for `assign-role`), it has to run from a
  root-owned binary and a root-owned unit file. Today Murph's daemon is a systemd
  user unit that any process in the operator's account can edit. Rally's doc gives
  the steps for moving to a system unit that still runs as `joshd`.
- Option A's private key is `/etc/muster/approval_key` (root, 0600). Option B's never
  leaves the Enclave.

## Reuse

Each verifier keeps the nonces it has accepted until their expiry passes. The catch
is that the ledger is written by the verifier, and a verifier running as the
operator's account (the Lamplighter, the daemon) keeps its ledger where a session can
delete it. So a single approval could be replayed **within its expiry window**, for
**the same action and target**. Three things limit this:

- Expiry is at most 10 minutes, and the default is 5.
- Most actions are idempotent for their exact target. Restarting onto the same commit
  twice is harmless.
- For `merge` and `assign-role`, the daemon's own state already makes a repeat a
  no-op: a merged PR stays merged, and a role is already held.

A root-owned ledger (written through `sudo muster-approve redeem`) would close this
fully, at the cost of a sudo prompt on every use. Recommendation: accept the bounded
window and say so in the threat model.

## Standing approvals

For example: "restarts to a commit on main that passed CI". These go in
`/etc/muster/standing-approvals.toml`, owned by root with mode 0644. A session can
read it but not change it.

```toml
[[allow]]
action = "restart-daemon"
host = "murph"
repo = "Castor-o7/muster"
require = ["on-main", "ci-green"]   # verifier checks these itself, via the GitHub API
until = "2026-12-31"
```

The verifier checks each `require` itself, never from a session's word: that the
commit is reachable from `origin/main`, and that the commit's checks are green. It
does this through a read-only token (MUS-12). A standing approval never covers
`assign-role` or `settings-edit`.

## How MUS-51 uses it: the watchman role

MUS-51 wants the roster to say who the watchman is, set only by the operator, and
cleared when that session ends. It can't be a `Session` field, because the gate lets a
machine write its own row whole.

1. The operator signs `assign-role` with target `watchman:<session id>`.
2. Any session submits it with a new command, `AssignRole { approval }`. The
   submitting session doesn't matter, because the approval is the authority.
3. The daemon verifies it (signature, exact target, expiry, nonce) and writes a
   `RoleAssignment { id: <role>, session, assigned_by, assigned_at, approval_nonce }`
   entity. Like `WakeWatcher` and `Succession`, only the daemon writes it: the gate
   refuses a client's events on it. A test must show that a client event setting it
   is refused, and that a session writing its own `Session` row doesn't change it.
4. The daemon deletes the assignment when the session is reaped, or when a newer
   `assign-role` for the same role is accepted.
5. The roster, `muster-shim roster` and the UIs show the role as
   **operator-assigned**, naming who assigned it and when. A role declared with
   `MUSTER_IDENTITY` stays shown as declared.

The role gives no authority. Restarts, merges and device changes still need their own
approval. A restarted watchman (MUS-3 keeps its nickname) needs assigning again, by
design: a new approval is cheap with Touch ID.

## What changes in the threat model

- "The operator's account is one trust zone" becomes: one trust zone **for
  everything except approvals**. A session still can't produce a valid approval,
  even though it can read everything the operator's account can.
- New gap: an approval can be reused within its expiry window for its exact action
  and target, as described in "Reuse" above.
- New gap: the signing prompt doesn't show the statement, so a session can put a
  different statement in front of the operator (see above).
- New out-of-scope item: someone who has both the operator's account and their
  fingerprint or sudo password. That person is the operator.

## Done when (from the roadmap)

A test shows each of the following:

- A relayed approval with a valid signature is accepted for exactly its action and
  target.
- A forged, expired, reused (within the ledger), or mismatched approval is refused.
- A public key writable by the operator's account is refused.
- A session on the operator's account can't produce a valid approval. For option A
  that means sudo refused without a password; for option B there is no private key
  to read.

## Questions for the operator

1. Option A, option B, or both? The recommendation is B, with a new Secretive key, as
   primary and A as fallback.
2. Is the bounded reuse window acceptable, or should every use pay for a root-owned
   ledger?
3. Which standing approvals do you want on day one, if any?
4. Is the starting action list right? Should `merge` be in it, or does a merge stay
   on your direct word?
