# Build progress

Updated 2026-11-13 13:35 (Asia/Kolkata).

- [x] Phase 1 — Rust workspace, typed bridge, UI foundation, empty browser adapter, design assets, CI baseline.
- [ ] Phase 2 — Ed25519 identity and signed mDNS discovery are wired into the desktop app; identity and SQLCipher keys use the OS credential store. Message transport, peer trust-on-first-use, online heartbeats, reconnect, and end-to-end delivery remain.
- [ ] Phase 3 — Encrypted local message history and persistent core outbox are present. Seeded conversations and fake chat content were removed; unsupported message sending, groups, calls, and file actions report unavailable. Real network messaging, group membership, receipts, edits/reactions synchronization, and file transfer remain.
- [x] Phase 4 — Native dock and compact windows are implemented, draggable with saved position, edge snapping, click-through hit testing, bubble timeout setting, and click-to-open compact behavior. Needs hands-on Windows/macOS validation.
- [ ] Phase 5 — Main screens, profile editing, and settings UI are present. Profile and preferences persist locally; fake history and fabricated media/call stats were removed; unsupported settings and actions are disabled. Profile photo sync, real group UI, localization, and full accessibility review remain.
- [ ] Phase 6 — Call layouts remain unavailable; signaling, WebRTC, group calls, PiP, and screen sharing are not connected.
- [ ] Phase 7 — Tauri updater is configured for signed GitHub Release artifacts, the update check/install flow is wired, and the release workflow builds Windows x64 plus Apple Silicon macOS. Encrypted backup primitives exist; database backup/export/restore, tray extras, hotkeys, and i18n remain.
- [ ] Phase 8 — Rust/UI CI checks pass locally. GitHub Actions run 43389740873 is building the Windows and macOS matrix. Code signing/notarization, hosted release build verification, security fuzzing, performance and accessibility passes remain.
- [ ] Phase 9 — Linux/mobile stubs and legacy import were not added.

Public repository: https://github.com/mhdanshadpm/AlverChat. `main`, the previous local branches, and the existing checkpoint tag are pushed. The app is not production-ready until the remaining network, messaging, and platform work is completed.
