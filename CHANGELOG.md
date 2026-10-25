# Changelog

## Unreleased

- MCP: the daemon itself keeps only the declared fields of an app's call (no `mentions`, `ring` or `audio`, history
  pages capped), so "no pings" no longer depends on the MCP server being the one that talks to it.
- MCP: when the agent cancels a call (or disconnects) the daemon drops it from the approval queue instead of asking you
  about something nobody waits for; one app can have at most 8 calls waiting for approval at a time.
- MCP: a dead daemon and a revoked token now give different, clear errors, and `tools/list` fails with a message instead
  of silently returning no tools.
- MCP: a line with invalid UTF-8 no longer ends the server, and a failing random generator no longer leaves a request
  unanswered. `tests/mcp-protocol.js` checks the protocol against a sandbox daemon.

- The agent list offers only what is found on the PC (plus "show the settings"); Claude Desktop from the Microsoft
  Store is now detected, and its config is written where that app reads it (inside the package folder).

## 0.1.1 - 2026-10-25

- `setup` is now a full-screen wizard (arrow keys, Space to toggle, Enter to confirm): system check, options,
  installation, connecting to Discord (with a one-key restart if Discord did not attach), choosing AI agents and rights,
  and a result page. `--yes`, `--plain` or a missing terminal keep the old line-by-line mode.
- Yes/no questions read key presses (Unicode) instead of bytes: Cyrillic `д`/`н` no longer ends with
  "stream did not contain valid UTF-8".
- `tests/tui-sandbox.ps1` runs the wizard against a fake Discord; `DISCORDCTL_TUI_KEYS` plays key presses for checks.
- `setup`/`install` find Discord in the registry uninstall entry when it is not in `%LOCALAPPDATA%\Discord`, and say so
  when only PTB or Canary is installed.
- `install.ps1` offers to download and run the official Discord installer (signature checked) when Discord is missing,
  and passes `DISCORDCTL_ARGS` to `setup` for unattended installs.
- `install.ps1` no longer calls the GitHub API (60 anonymous requests an hour per IP, which people on a shared IP hit):
  it resolves the latest tag from the `releases/latest` redirect and downloads files by their fixed URLs.
- The launcher refuses to start a `*.real.exe` that is itself a launcher (it used to re-spawn itself without end), and
  `doctor`/`install` report such an install as broken.

## 0.1.0 - 2026-10-25

First public version.

- Launch shim for `Update.exe` and the root `Discord.exe` stub; daemon that owns Discord and its debugging pipe.
- Optional `discord://` link handler that follows Discord's rewrites (`install --link-hook`, asked in `setup`; off by default).
- Reading: servers, channels, chats, friends, message history with pagination.
- Writing: send, reply, edit, delete, react; rate limits; no pings by default; `--dry-run`.
- Voice: join, leave, mute, deafen, status. Screen/window sharing.
- Rights for apps and agents: tokens, rights by id, per-call approval, audit log.
- MCP server with permission-aware tools and untrusted-data marking.
- `setup` wizard, `install`/`uninstall`, `doctor --fix`, `mcp install` for Claude Code, Claude Desktop and Cursor,
  shell completions.
- Fewer antivirus false positives: install, uninstall and `doctor` no longer spawn `powershell`, `tasklist` or `reg` (direct Win32 calls instead),
  version info and an `asInvoker` manifest in every exe, `discord://` hook is opt-in.
