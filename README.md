# discordctl

**Your Discord, from the terminal and from your AI agents.**
Read and write messages, manage voice calls, share a window, and connect Claude Code, Claude Desktop or Cursor through
MCP, with rights you approve line by line.

[Русская версия](README.ru.md) · [Architecture](docs/ARCHITECTURE.md) · [Security](SECURITY.md)

> [!WARNING]
> discordctl automates **your own user account** through the desktop client. That is against Discord's Terms of Service
> and can get an account banned. Reads and writes are rate-limited and writes never ping anyone by default, but there is
> no guarantee. You use this at your own risk. It is not affiliated with Discord.

Windows 10/11 and the official Discord desktop app are required.

## Install

**One line (PowerShell):**

```powershell
irm https://raw.githubusercontent.com/zalikuska/discordctl/main/scripts/install.ps1 | iex
```

It downloads the latest release, verifies its SHA-256, and runs `discordctl setup`, which asks before it changes anything.
Prefer to look first? Download `discordctl-*-windows-x64.zip` from [Releases](https://github.com/zalikuska/discordctl/releases),
unpack it, and run `discordctl.exe setup`.

**From source** (Rust 1.80+, Node 20+ only if you change `payload/`):

```powershell
git clone https://github.com/zalikuska/discordctl && cd discordctl
cargo build --release
.\target\release\discordctl.exe setup
```

`setup` walks through five steps: checks your Discord, installs a small launcher in front of `Update.exe` and
`Discord.exe` (originals are kept), starts Discord under discordctl, runs a health check, and offers to connect an AI
agent. Everything is reversible with `discordctl uninstall`.

> The programs are unsigned and hook the launch of another program, so antivirus software may complain. Install and uninstall start no
> `powershell` and change `discord://` links only if you ask (`--link-hook`). `discordctl doctor`
> tells you what to do ([details](#troubleshooting)).

## Quick start

```powershell
discordctl chat                           # your direct chats and groups
discordctl read @alice -n 20              # last 20 messages
discordctl send @alice "hello" --dry-run  # preview; drop --dry-run to send
discordctl voice join "My Server/Lounge"  # join a voice channel
discordctl stream list                    # windows you could share
discordctl doctor                         # is everything healthy?
```

### Targets: one syntax everywhere

| You write | Means |
|---|---|
| `@alice` or `"@Group name"` | a direct chat or group, by name, nickname or member |
| `"My Server/general"` | a channel in a server |
| `123456789012345678` | any id |
| `@*`, `"My Server/*"`, `*` | (rights only) all chats, a whole server, everything |

## Commands

```text
setup | install | uninstall | doctor | status | start         lifecycle
server | channel list <server> | chat | friend                  what you have
message read|send|edit|delete|react <target> …                  (shortcuts: read, send)
voice status|join|leave|mute|unmute|deafen|undeafen
stream list|status|start|stop
app add|list|remove|log|approve|whoami                          rights of apps and agents
mcp install|status|remove                                       connect AI agents
daemon start|stop · completions <shell>
```

Every command has `--help`, and `--json` for scripts. `--no-color` (or `NO_COLOR`) switches colors off.
Shell completion: `discordctl completions powershell | Out-String | Invoke-Expression`.

## AI agents (MCP)

```powershell
discordctl mcp install                    # pick agents and a preset; asks you to approve the rights
discordctl app approve                    # keep open in another window: writes wait here for your OK
```

Presets: **read-only** (list and read) and **assistant** (also send/edit/delete/react, each one approved by you).
Voice and window sharing are granted explicitly with `discordctl app add`.

- The agent gets a **token with only the rights you approved**; revoke it at once with `discordctl mcp remove` or
  `discordctl app remove <name>`.
- The MCP server refuses to run without a token, shows the agent only the tools it may use, and marks everything that
  came from Discord as untrusted data so other people's messages are less likely to steer the model.
- Every decision is in the audit log: `discordctl app log`.

> [!IMPORTANT]
> A token limits the *MCP connection*. It does not limit an agent that also has a normal shell as you; such an agent can
> run `discordctl` as the owner. Give agents MCP, not your shell, if the boundary matters. See [SECURITY.md](SECURITY.md).

## How it works, in short

discordctl starts Discord itself through a private Chromium debugging pipe (no open port), injects a small script into
the client, and uses the client's own code for everything: its stores for reading, its HTTP module for the API (so
requests look like the client's own), its actions for voice. A background daemon owns the connection and enforces rights.
Details: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Troubleshooting

Run `discordctl doctor` first; it explains every problem and `discordctl doctor --fix` repairs what is safe to repair.

- **Discord is open but not attached.** It was started before the install or outside the launcher. Quit it completely
  (also from the tray) and open it again.
- **Self-check fails after a Discord update.** Discord renamed something internal. Reading may still work; the payload
  needs an update ([how](docs/ARCHITECTURE.md#adapting-to-a-new-discord-build)).
- **After rebuilding from source** run `discordctl install` again: the daemon runs from `%LOCALAPPDATA%\discordctl\bin`.
- **Antivirus (for example Kaspersky) quarantines or blocks a file.** Restore it, add `%LOCALAPPDATA%\discordctl` and the
  folder you unpacked/built into to the trusted list, then `discordctl doctor --fix`.

## Uninstall

```powershell
discordctl uninstall            # puts Discord back; a running Discord is not closed
discordctl uninstall --purge    # also deletes tokens, audit log and logs (asks first)
```

## Development

```powershell
cargo test --release
powershell -File tests\e2e-shim.ps1     # sandbox tests with a fake Discord
powershell -File tests\e2e-ops.ps1
cd payload; npm ci; npm run build        # after editing payload/src (the bundle is committed)
```

See [CONTRIBUTING.md](CONTRIBUTING.md). Licensed under the [MIT license](LICENSE).
