# Architecture

discordctl controls the **official Discord desktop client** (Windows) from the outside. It does not use the bot API or a
user token: it drives the client's own code through the Chromium debugging protocol (CDP), the same way DevTools does.

```
 terminal / scripts ──┐                      ┌── Discord (Electron)
                      ├─ named pipe ─ daemon ┤     ├─ renderer: payload.js (our code, injected)
 AI agent ── MCP ─────┘   (rights, audit)    └─────┘     └─ client's stores, HTTP module, actions
```

| Crate / dir | Role |
|---|---|
| `crates/core` | Paths, IPC protocol, permissions model, launch shim rules, `discord://` registry handler |
| `crates/shim` | Tiny launcher that replaces `Update.exe` and the root `Discord.exe` stub; asks the daemon to start Discord |
| `crates/daemon` | Owns Discord and its CDP pipe; injects the payload; enforces rights; confirmations; audit log |
| `crates/cli` | `discordctl`: installer, `doctor`, chat/voice/stream commands, app rights, MCP setup |
| `crates/mcp` | `discordctl-mcp`: MCP server (stdio), a thin client of the daemon API |
| `payload/` | TypeScript bundle that runs **inside** the Discord renderer; built to `payload/dist/payload.js` (committed) and embedded into the daemon |
| `tools/fake-discord` | Stand-in for Discord used by the sandbox end-to-end tests |

## Transport: a pipe, not a port

`--remote-debugging-port` has no authentication: any local process could take over the account. discordctl uses
`--remote-debugging-pipe` instead: two anonymous pipe handles that only the child process inherits (passed with a
`PROC_THREAD_ATTRIBUTE_HANDLE_LIST`). Only the process that started Discord can talk CDP to it, and that process is the daemon.

Consequences:

- Discord must be started **by the daemon**. Closing the pipe kills Discord, so the daemon lives as long as Discord does.
- The Chromium child processes re-run `Discord.exe`, so the real binary can't be replaced. Instead the shim replaces the
  two *entry points* Squirrel uses: `Update.exe` (taskbar, autostart, restart after updates) and the root `Discord.exe`
  stub (Start menu and desktop shortcuts). Originals are kept as `Update.real.exe` / `Discord.real.exe`.
- The shim **always falls back** to the real program on any error or on arguments it does not whitelist, so it cannot
  prevent Discord from starting. Debug flags such as `--remote-debugging-port` are refused.
- Discord's updater overwrites the root stub several times per start; the daemon watches the folder
  (`ReadDirectoryChangesW`) and restores the shim immediately, with retries.
- Optional (`--link-hook`): `discord://` links are pointed at the root stub (registry, HKCU) and the daemon restores the value whenever Discord
  rewrites it (`RegNotifyChangeKeyValue`).

## Injecting the payload

For the main window (a `discord.com` page, not popouts) the daemon enables `Page`, adds the payload with
`Page.addScriptToEvaluateOnNewDocument` and reloads the page. The payload:

1. hooks `window.webpackChunkdiscord_app` and its `push` to obtain every webpack `require`; the runtime with the largest
   module cache is the main one;
2. finds Flux **stores** by `getName()` + `addChangeListener` (matching by method names alone hits i18n proxies);
3. finds the client's **HTTP module** by probing candidates with `GET /users/@me` (the real one returns the current user);
   requests then carry the client's own headers and token, no separate `fetch`;
4. finds **action creators** with minified names by unchanging strings in their source (`findFnBySource`), for example
   the function that mentions `startStreamWithSource`;
5. exposes `window.__discordctl.call(method, params)` which always returns a JSON string.

`discordctl doctor` runs a **self-check** (`selfcheck`): which stores, methods and action modules exist. When Discord
renames something, this says exactly what broke.

## Safety model

**Rate limits.** Requests are queued one at a time with jitter; writes are slower and capped at 15 per minute. A `429`
waits for `retry_after`. A timed-out write is never retried (a duplicate is worse than an error).

**Writes never ping anyone** unless asked (`allowed_mentions` is empty by default).

**Calls to people.** Joining a call in a chat with no active call would ring everyone; it needs `--ring`, which AI agents
don't get at all.

**Apps and rights.** Without a token a client is the *owner* (the CLI). With `DISCORDCTL_TOKEN` it is an *app* with a
manifest of rights:

- rights are stored by **id**, never by name; names are resolved at approval time and shown to you;
- areas: `list`, `read`, `write`, `voice` (+ `voice_channels`), `stream`; `read` does not imply `write`;
- the daemon resolves the target itself, checks **that id**, and executes with **that id** (no check/use gap);
- `write`, `voice` and `stream` calls wait for a human answer (`discordctl app approve`) with the exact text or window
  title shown; silence for two minutes is a refusal;
- tokens are 256-bit random values, only their SHA-256 is stored; revocation is immediate and also cancels waiting calls;
- an audit log records decisions (never message text).

**MCP.** `discordctl-mcp` refuses to start without a token, only exposes tools the app is allowed to use, whitelists
tool arguments, caps history pages, and wraps everything that came from Discord in a `<discord-data-ID>` block with a
random id generated *after* the data arrived, plus a notice that it is untrusted. This reduces prompt-injection risk
from other people's messages; it cannot remove it, which is why writes stay behind human approval.

**Limits of this model (please read).**

1. A token restricts the process it is given to (an MCP server, a script). It does **not** stop an agent that also has a
   normal shell as your user: that agent can call the CLI as the owner, read Discord's own token from disk, and so on.
   Connect agents through MCP and keep them away from your shell if the boundary matters.
2. The terminal dialogs read from the real console (`CONIN$`) and refuse piped input, which stops scripts and shells
   without a terminal, but the daemon cannot tell a human from an agent with a pseudo-terminal. A real boundary would
   need approval outside the agent's reach (a UAC prompt, a separate Windows user).
3. Owner-level requests on the pipe (`dev call`, `dev reload`, `start`) are open to any process of your user.
4. Automating a user account violates Discord's terms of service. That risk is yours.

## Voice and screen sharing

Voice uses the client's own action creators (`selectVoiceChannel`, `disconnect`, `toggleSelfMute`, `toggleSelfDeaf`),
so the client does its usual permission checks. No audio is read or sent by discordctl.

Screen/window sharing lists sources through the Electron bridge (`DiscordNative.desktopCapture`) and starts the share by
calling the function the client invokes **after** you pick a source in its dialog. The dialog is bypassed; the client's
checks (must be in a voice channel, screen-recording permission, quality limits) are not. Sharing starts without sound.

## Operations

`discordctl install` / `uninstall` are idempotent and reversible. Uninstall first turns the daemon's watching off, then
restores files (otherwise the daemon sees the changes as an updater overwrite and reinstalls the shim). A running
Discord is never closed; the daemon exits by itself once Discord has closed and the shim is gone.

Logs (`daemon.log`, `shim.log`) rotate at 1 MB.

## Adapting to a new Discord build

Discord changes internals without notice. When `doctor` reports a failed self-check:

1. `discordctl dev call selfcheck` shows what is missing.
2. Use the diagnostics in `payload/src/methods.ts` (`describe`, `find_modules`, `find_fn_names`, `find_fn_src`,
   `module_fns`) through `discordctl dev call …` to locate the new names; prefer matching by stable strings.
3. Edit `payload/src`, `npm run build`, then test without restarting Discord using
   `discordctl dev reload --payload payload/dist/payload.js`.
4. Rebuild Rust (`cargo build --release`) so the new bundle is embedded, and run `discordctl install`.

## Tests

| Suite | What it covers | Needs |
|---|---|---|
| `cargo test` | Argument whitelist, permissions, CDP framing, MCP wrapper/tools, CLI helpers | – |
| `tests/e2e-shim.ps1` | Shim install, launch, forwarding, fallbacks, repair, uninstall (fake Discord) | Windows |
| `tests/e2e-ops.ps1` | Install, doctor, `discord://` handler, uninstall (fake Discord, test registry key) | Windows |
| `tests/e2e-setup.ps1` | `setup` wizard, help, completions, MCP commands (fake Discord) | Windows |
| `tests/e2e-auth.ps1`, `tests/mcp-smoke.js`, `tests/live-read.ps1` | Rights, MCP, reading, against your live Discord | running, attached Discord |
| `tests/live-write.ps1`, `tests/live-stream.ps1` | Really send / really share (only with `-Yes`, in a chat or channel you choose) | live Discord |
