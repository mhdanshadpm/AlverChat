# Build progress

Updated 2026-11-13 (Asia/Kolkata).

- [x] Phase 1 — workspace, CI, tokens, bridge, mock mode, component gallery (2026-11-13 09:36 Asia/Kolkata)
- [ ] Phase 2 — core identity, security, discovery, transport, storage, outbox (in progress 2026-11-13 09:42 Asia/Kolkata: identity/signatures, Argon2id/XChaCha helper, SQLCipher schema/FTS/outbox API, protocol/HLC with size/version validation, mDNS advertisement/browse, signed UDP beacon; TLS WebSocket and keychain wiring remain)
- [ ] Phase 3 — messaging, groups, files, search (in progress 2026-11-13 09:55 Asia/Kolkata: mock chat search, send, custom context menu, reply/edit/local-delete/tombstone, People/Groups lists; persistent Rust message primitives and outbox are available)
- [ ] Phase 4 — dock, floating icon, bubbles, compact window
- [ ] Phase 5 — full UI screens and interactions
- [ ] Phase 6 — calls and screen sharing
- [ ] Phase 7 — backup, updater scaffolding, tray, hotkey, i18n
- [ ] Phase 8 — hardening, accessibility, packaging, docs
- [ ] Phase 9 — optional legacy/mobile stubs

Screens implemented as mock UI: full chat, people, groups, call history, profile, three-step onboarding, general settings, component gallery. Interactions include chat search, send, context menu, reply/edit/delete, theme, and profile edits. Remaining: announcements, floating dock/bubbles, compact window, detailed settings, actual group/file/call actions and WebRTC. This is a foundation build; later phases are not yet complete.
