# AlverChat progress report

Updated 2026-11-13 (Asia/Kolkata). The product is still incomplete and should not be described as production-ready.

## Implemented

- Tauri 2 desktop shell with native always-on-top dock and compact windows, saved draggable positioning, edge snapping, transparent hit testing, and click-to-open compact behavior.
- Removes seeded demo peers, messages, fabricated call history, and fake media details. Empty states use live peer/message counts. Unsupported messaging, calls, group creation, and settings are disabled or report unavailable.
- Ed25519 identity persisted through the system credential store; signed mDNS service advertisements are verified before showing discovered peers.
- SQLCipher message database with its key stored in the OS credential store, ordered local history, and persistent core outbox primitives.
- Profile/theme/preferences persistence on this device. Bubble auto-hide and enable preferences now refresh in the overlay window when changed; dock snapping is connected to native behavior.
- Tauri signed updater configured for GitHub Releases. The About page can check for an update, ask before install, download it, and relaunch. A GitHub Actions release workflow builds Windows x64 and Apple Silicon macOS artifacts on `v*` tags.
- Public repository: https://github.com/mhdanshadpm/AlverChat. The private updater signing key is stored in GitHub Actions secrets; only its public key is committed.

## Still incomplete

- Signed discovery currently does not have an active transport listener. Messaging is explicitly unavailable; there is no TLS/WebSocket, TOFU trust store, peer heartbeat, reconnect, send/receive, or outbox delivery integration.
- Groups, file transfer, receipts, message synchronization, calls/WebRTC, screen sharing, incoming call UI, profile/avatar sync, announcement sync, backup/export/restore, and most notification/call/privacy settings are not implemented.
- Hosted CI run 43389740873 passed UI and macOS; Windows Clippy is still running. A follow-up checks the latest source. Native window behavior has not been visually validated in this session; the computer-use bridge returned `noWindowsAvailable`.
- macOS notarization and Windows/macOS code signing certificates are not configured. Release installers will require platform trust setup before broad public distribution.

## Local validation

Completed successfully on macOS:

- `cargo test -p alver-core` — 25 tests passed.
- `cargo test -p alverchat-desktop` — 6 tests passed.
- `cargo clippy -p alverchat-desktop -- -D warnings`.
- `cargo check -p alverchat-desktop` and `cargo fmt --all`.
- `pnpm lint`, `pnpm test` — 5 tests passed, and `pnpm build`.
- `pnpm exec tauri build --debug --no-bundle`.

## Commands

```sh
pnpm install --frozen-lockfile
pnpm dev
pnpm dev:overlay-check
pnpm test
pnpm lint
pnpm build
cargo fmt --all -- --check
cargo test -p alver-core
cargo test -p alverchat-desktop
cargo clippy -p alverchat-desktop -- -D warnings
```

## Manual checks still required

On real macOS and Windows devices, validate dock dragging, click-through and compact-window positioning, startup, keychain access, mDNS peer discovery, permissions, updater install/rollback behavior, and installer handling. Message delivery, groups, files and calls cannot yet be manually tested because their transports are not implemented.
