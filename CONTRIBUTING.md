# Contributing

Thanks for helping! discordctl is Windows-only and drives the official Discord client, so a few things are different
from a typical project.

## Set up

```powershell
git clone https://github.com/zalikuska/discordctl; cd discordctl
cargo build --release
cargo test --release
cd payload; npm ci; npm run build        # only if you touch payload/src
```

Requirements: Rust 1.80+, and Node 20+ if you change the TypeScript payload. The built bundle
`payload/dist/payload.js` is committed (and embedded in the daemon), so CI fails when it is out of date: rebuild it
and commit the result.

## Layout

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md). In short: `crates/core` (shared), `crates/shim`, `crates/daemon`,
`crates/cli`, `crates/mcp`, `payload/` (runs inside Discord), `tools/fake-discord` (test double).

## Testing

| What you changed | Run |
|---|---|
| Anything in Rust | `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --release` |
| Shim, install, doctor, `discord://` | `powershell -File tests\e2e-shim.ps1` and `tests\e2e-ops.ps1` (sandboxed: fake Discord, test registry key) |
| Rights, MCP, payload | with an attached Discord: `tests\e2e-auth.ps1`, `node tests\mcp-smoke.js`, `tests\live-read.ps1 -Channel @someone` |
| Writing / voice / sharing | `tests\live-write.ps1`, `tests\live-stream.ps1` really send or share: use your **own** empty server, never someone else's chat |

Never run live tests that write, call or share in chats where other people are.

## Working on the payload

Discord's internals are minified and change without notice. Prefer finding things by **stable strings** (store names,
Flux action types, analytics labels) over exported names. Use the diagnostics in `payload/src/methods.ts` through
`discordctl dev call …`, and iterate without restarting Discord with
`discordctl dev reload --payload payload/dist/payload.js`. Keep `selfcheck` honest: add anything new that voice, stream
or reading depends on.

## Style

- Rust: `cargo fmt` (see `rustfmt.toml`), no warnings from clippy.
- User-facing text is English. Many existing code comments are Russian; new comments may be either, but explain *why*.
- Keep the safety rules intact: nothing sends, rings or shares by default, writes are rate-limited, rights are by id,
  and tests print structure, not other people's messages.

## Pull requests

Small, focused changes with a note on how you tested (including the output of `discordctl doctor` if it touches
install or compatibility). Mention your Discord build if the change is about internals.
