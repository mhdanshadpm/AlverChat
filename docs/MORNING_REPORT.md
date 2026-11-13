# AlverChat morning report

Updated 2026-11-13 (Asia/Kolkata). This report describes the current build checkpoint; the requested nine-phase product is **not complete**.

## Done

- Created the Tauri 2 / Rust workspace, React 18 / TypeScript UI, typed bridge boundary, mock mode, component gallery, CSP, bundled fonts, and design-board crops/color samples.
- Built light/dark chat, People, Groups, Calls, Settings, profile and onboarding screens, floating icon/bubbles, quick-chat drawer, context menus, announcements, and call-window previews. These visible peer, message, group, and call experiences remain mock-backed.
- Added Rust primitives for Ed25519 identity, Argon2id/XChaCha payload encryption, SQLCipher storage and FTS, persistent outbox, signed discovery beacons, versioned envelopes/HLC, signed and persisted group rosters, sender-only message edits/tombstones, bounded FTS search, BLAKE3-verified file reception, and an authenticated encrypted backup container for opaque payload bytes.
- Added tests for crypto, storage, protocol, group authorization/epochs, message tombstones, file bounds/hash/path handling, and backup authentication.

## Incomplete, skipped, and blocked

- **Phase 2:** TLS WebSocket integration, TOFU certificate verification, OS keychain persistence, outbox replay, peer trust management, and the two-peer headless demo are not connected. A first transport attempt was removed after three build iterations and recorded in [BLOCKERS.md](BLOCKERS.md). The UI still uses mock data in the desktop shell.
- **Phase 3:** UI group creation is session-local; group APIs are not wired through the bridge. Live messaging, receipts, typing, reactions, pins/stars/forwarding, file transport/pause/resume, and voice notes are incomplete.
- **Phase 4:** Floating icon and bubbles are rendered inside the app UI; there are no native always-on-top dock/compact/menu windows, native hit testing, tray, or global hotkey.
- **Phase 5:** Main screens and visual states exist, but several actions are presentation-only. Profile photos, group info/permissions, persisted settings, full accessibility review, and responsive mobile layout are incomplete.
- **Phase 6:** Voice/video/group call layouts are previews only. Signaling, WebRTC, PiP, incoming-call flow, and screen sharing are not connected.
- **Phase 7:** Only an encrypted opaque-payload backup helper exists. Database/avatar export and restore, updater, tray, hotkey, launch-at-login, and i18n are incomplete.
- **Phase 8:** No Windows build/installer, macOS signed package, screenshot comparison suite, performance pass, security fuzzing, or full accessibility audit has been completed.
- **Phase 9:** No legacy importer or Linux/mobile target stubs have been added. No `/legacy` directory was present at initial inspection.

## Decisions and design deviations

- The initial workspace was empty, so a Vite root with shared packages and a Tauri shell was created.
- Mock data stays available so the screens can be reviewed offline. The UI's “connected” and peer counts are illustrative and do not indicate live LAN connectivity.
- Group creation currently adds a group only for the current UI session. The avatar palette is preserved locally; signed group state is available in the Rust core but not wired to that action.
- Setup, calls, announcement acknowledgements, settings, and call states are visual previews and are not persisted or sent over the LAN.
- Profile/contact illustrations use initials and gradients rather than bundled photographs. The UI uses CSS for most styling; the requested Radix, Zustand, Framer Motion, TanStack Virtual, and i18next packages are present but are not comprehensively integrated.

## Manual checks already performed

- On macOS, opened the dev UI and reviewed chat, context menu/reply, Settings, call preview, Groups, and the new-group flow. Created a sample group and verified that selecting a slate avatar color carries through to the chat header and details panel.
- UI automation remains a local review aid; it does not verify native window behavior, network exchange, or production permissions.

## Hand tests required on real systems

- **Windows 10/11:** Install and launch; verify firewall prompts and LAN discovery on two machines; send/reconnect messages; test scaling and multi-monitor movement; check tray/global hotkey, topmost overlay click-through, compact/main windows, and WebView2 offline packaging.
- **macOS 12+:** Install and launch; verify Local Network, camera, and microphone permissions; test two-machine discovery and reconnect; check native traffic lights, menu bar behavior, Spaces/full-screen overlay, monitor movement, and universal package/signing behavior.
- On both systems, verify identity persistence, TOFU key-change warnings, encrypted storage/keychain recovery, edit/delete synchronization, file transfer/hash failure and resume, backup/restore, voice/video/group calls, PiP, and screen sharing. These flows are not implemented yet and should not be treated as passing.

## Commands

```sh
pnpm install
pnpm dev
pnpm dev:peers 2
pnpm build
pnpm test
pnpm lint
pnpm tauri dev
cargo fmt --all -- --check
cargo test -p alver-core
cargo clippy -p alver-core -p alver-platform -- -D warnings
cargo check -p alverchat-desktop
cargo build --workspace
```

`pnpm dev:peers 2` opens two local mock UI instances only; it does not test LAN traffic. Windows packaging, macOS signing/notarization, and real network/call commands remain to be added and verified.
