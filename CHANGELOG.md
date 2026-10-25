# Changelog

## Unreleased

- Events review fixes: a typing or message event of a server channel that arrives without a server id is no longer taken
  for a direct chat (it could match a `dm:*` right); events say why they are not flowing when the client's event
  dispatcher cannot be set up; `presence --status` with a wrong status is an error instead of "everybody". The
  declared minimum Rust version is now 1.82 (the code already needed it).
- Presence: `discordctl presence [user] [--server S] [--status online] [--activity game]` and the MCP tool `discord_presence`
  show who is online and what they are doing (new `list` word `presence`). In events it is opt-in: `discordctl watch
  presence --activity minecraft --status online --user alice` (MCP: kind `presence` with `filter`) reports when somebody
  starts or stops a matching activity or enters a status. `all` never includes presence.
- Live events: `discordctl watch [messages|voice|typing] [-c chat] [-n N] [--json]` and the MCP tool
  `discord_wait_events` deliver new, edited and deleted messages, voice joins/leaves/moves/state changes and typing as
  they happen. The payload subscribes to the events the Discord client already receives (nothing extra is requested),
  the daemon filters them per listener by rights (`read` for chats, the `voices` list word for voice), keeps up to 500
  per listener between calls and subscribes only while somebody listens. It works against a real Discord only once
  tried there: event shapes can change between Discord builds (`selfcheck` reports whether the dispatcher is found).

## 0.1.3 - 2026-10-25

- Russian interface: `discordctl lang ru` (or `en`, or `auto` for the Windows display language) is remembered; `--lang ru`
  and `DISCORDCTL_LANG=ru` override it for one run (for the one-line installer: `DISCORDCTL_ARGS="--lang ru"`). Messages,
  `--help`, `doctor` and the wizard are translated; in the wizard Ctrl+L switches the language on the spot. `--json`
  output and command names stay English. Text without a translation is shown in English.
- Who is where: `discordctl voice who <Server/channel>` or `voice who --server <name>` shows who is in a voice channel
  (or in every occupied voice channel of a server), and `discordctl server members <server> [-f name] [-n 100]` lists
  the members of a server. Members are the ones the Discord client has already loaded: big servers load gradually, so
  the answer says `loaded` and `total_members`. MCP gets the same as `discord_voice_who` and `discord_list_members`,
  opened by the new `list` words `voices` and `members` (included in both presets and in `*`).
- MCP timeouts (seen in Cursor): a call that waits for your approval now gives up after 55 seconds with a clear
  message ("answer with `discordctl app approve`") instead of 120, because agent clients drop a call after about a minute
  and showed only "timeout"; while it waits the server sends `notifications/progress` every 10 s (when the client sent a
  `progressToken`) so clients that understand it keep the call alive. `tools/list` no longer blocks other requests, and
  starting the daemon is waited for 20 s instead of almost three minutes.
- New `discordctl update`: finds the newest release without the GitHub API, downloads it over the system's HTTPS (WinHTTP, no
  PowerShell or curl), checks its SHA-256 against `SHA256SUMS.txt`, and installs it, replacing even the running
  `discordctl.exe`. `--check` only looks, `--no-install` only downloads and unpacks, `--release v0.1.1` picks a version,
  `--yes` skips questions. A running attached Discord is closed (after asking) and started again.
- New MCP preset **autonomous**: write, voice and screen sharing with no approval prompts (`mcp install --preset
  autonomous`, or in the wizard). It is never the default, shows a red warning, and the confirmation button starts on
  "Don't allow". `discordctl mcp remove` takes it away.
- `doctor` no longer warns about autostart when the Run key starts Discord through the root `Discord.exe`
  (newer Discord builds write that): the shim sits there too.

## 0.1.2 - 2026-10-25

- MCP: the daemon itself keeps only the declared fields of an app's call (no `mentions`, `ring` or `audio`, history
  pages capped), so "no pings" no longer depends on the MCP server being the one that talks to it.
- MCP: when the agent cancels a call (or disconnects) the daemon drops it from the approval queue instead of asking you
  about something nobody waits for; one app can have at most 8 calls waiting for approval at a time.
- MCP: a dead daemon and a revoked token now give different, clear errors, and `tools/list` fails with a message instead
  of silently returning no tools.
- MCP: a line with invalid UTF-8 no longer ends the server, and a failing random generator no longer leaves a request
  unanswered. `tests/mcp-protocol.js` checks the protocol against a sandbox daemon.

- The agent list offers only what is found on the PC (plus "show the settings"); Claude Desktop from the Microsoft
  Store is now detected, and its config is written where that app reads it (inside the package folder). A leftover
  `%APPDATA%\Claude` folder after uninstalling no longer counts as "installed".
- Approvals: a call that was approved just as the app hung up is dropped instead of executed; a call waiting for
  approval is refused if the app was re-granted (new token or rights) in the meantime; app parameters of the wrong type
  (for example `"remove": "false"`) are rejected, so what you are shown is what runs. Pressing a key before the
  question appears no longer answers it.
- The launcher passes the real program's exit code through unchanged (codes above 255 were cut, crashes read as success).
- `Config` is written atomically; closing Discord no longer matches `DiscordPTB`/`Discord2` folders by name prefix
  (also in `install.ps1`); `install.ps1` waits for the Discord app folder, not just `Update.exe`, before continuing.
- Non-ASCII user names no longer break reading the Discord folder from the uninstall entry.
- The check and cross marks in the wizard and `doctor` are single-cell glyphs now: the old ones were drawn as wide
  emoji in Windows Terminal and broke the alignment of the `[x]` toggles.

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
