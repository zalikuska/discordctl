## What and why

## How I tested

- [ ] `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --release`
- [ ] `tests\e2e-shim.ps1` / `tests\e2e-ops.ps1` (if install, shim or doctor changed)
- [ ] Rebuilt `payload/dist/payload.js` (if `payload/src` changed)
- [ ] Live tests only against my own chats/servers

## Safety check

- [ ] Nothing sends, rings or shares by default; rights are still checked by id in the daemon
