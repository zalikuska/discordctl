# Security

discordctl is a tool that controls a Discord account. Treat it like you would treat access to that account.

## What is protected, and what is not

- **No open ports.** Discord is started with a private Chromium debugging *pipe*; only the daemon can talk to it.
- **The daemon's pipe** is accessible only to your Windows user and SYSTEM, and refuses remote clients.
- **App tokens** are random 256-bit values; only their SHA-256 is stored (`%LOCALAPPDATA%\discordctl\apps.json`).
  The token itself is shown once, and lives in the config of the agent you installed it into.
- **An app can only pass the declared fields of each call.** The daemon drops everything else (pings, ringing people,
  stream sound), so this holds for a program that talks to the pipe directly, not only through the MCP server.
- **Rights are stored by id** and every call is checked by the daemon, not by the MCP server or the client.
- **Prompt injection.** Messages are written by other people. The MCP server marks everything from Discord as untrusted,
  but a model can still be fooled. That is why writes, voice actions and sharing wait for your approval by default.

What is **not** protected (also described in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md#safety-model)):

1. A token restricts the *connection it is given to*. An agent with a normal shell running as you can call `discordctl`
   as the owner, read Discord's own token from disk, or edit discordctl's files. Don't hand such an agent a shell if
   the boundary matters.
2. The terminal approval dialogs refuse piped input but cannot tell a human from an agent with a pseudo-terminal.
3. Owner-level requests on the daemon pipe (`dev call`, `dev reload`, `start`) are open to any process of your user.
4. Binaries are unsigned. Verify the SHA-256 of release archives (`SHA256SUMS.txt`) or build from source.

## Reporting a vulnerability

Please do **not** open a public issue for a security problem. Use GitHub's private vulnerability reporting
("Security" tab → "Report a vulnerability") on this repository. Include the version (`discordctl --version`), what you
did, and what you expected. You should get an answer within a few days.

## Responsible use

Automating a user account violates Discord's Terms of Service. Use discordctl only on your own account, never to message
people who did not expect it, and never to harvest or republish other people's content.
