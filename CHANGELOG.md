# Changelog

## Unreleased

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
