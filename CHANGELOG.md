# Changelog

## 0.1.0 (unreleased)

First public version.

- Launch shim for `Update.exe` and the root `Discord.exe` stub; daemon that owns Discord and its debugging pipe.
- `discord://` link handler that follows Discord's rewrites.
- Reading: servers, channels, chats, friends, message history with pagination.
- Writing: send, reply, edit, delete, react; rate limits; no pings by default; `--dry-run`.
- Voice: join, leave, mute, deafen, status. Screen/window sharing.
- Rights for apps and agents: tokens, rights by id, per-call approval, audit log.
- MCP server with permission-aware tools and untrusted-data marking.
- `setup` wizard, `install`/`uninstall`, `doctor --fix`, `mcp install` for Claude Code, Claude Desktop and Cursor,
  shell completions.
