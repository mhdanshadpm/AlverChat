# Build progress

Updated 2026-11-13 10:43 (Asia/Kolkata).

- [x] Phase 1 — workspace, CI, tokens, bridge, mock mode, component gallery (2026-11-13 09:36 Asia/Kolkata)
- [ ] Phase 2 — core identity, security, discovery, transport, storage, outbox (in progress 2026-11-13 10:35 Asia/Kolkata: identity/signatures, Argon2id/XChaCha helper, SQLCipher schema/FTS/outbox API, protocol/HLC with size/version validation, mDNS advertisement/browse, signed UDP beacon; TLS WebSocket and keychain wiring remain. TLS WebSocket attempt deferred after three integration build failures; see BLOCKERS.md)
- [ ] Phase 3 — messaging, groups, files, search (in progress 2026-11-13 10:43 Asia/Kolkata: mock chat search/send, custom context menu, reply/edit/local-delete/tombstone, People/Groups lists, local group creation with member selection and avatar color; Rust core signs and atomically persists group rosters, checks admin updates and successor epochs, rejects stale updates, and protects Everyone group state. Persistent message edits keep revision history; sender-only tombstones remove FTS entries and prevent resurrection)
- [ ] Phase 4 — dock, floating icon, bubbles, compact window
- [ ] Phase 5 — full UI screens and interactions (in progress 2026-11-13 10:23 Asia/Kolkata: profile/onboarding, People/Groups, every settings category, context actions, LAN announcement/acknowledgement mock)
- [ ] Phase 6 — calls and screen sharing (call window previews for voice/video/group; media/signaling not connected)
- [ ] Phase 7 — backup, updater scaffolding, tray, hotkey, i18n
- [ ] Phase 8 — hardening, accessibility, packaging, docs
- [ ] Phase 9 — optional legacy/mobile stubs

Screens implemented as mock UI: full chat, people, groups (including local group creation with member selection and avatar color), call history, voice/video/group call previews, profile, three-step onboarding, all settings categories, LAN announcements, floating icon/bubbles, compact quick-chat drawer, component gallery. Interactions include chat search, send, context menu, reply/edit/delete, theme, and profile edits. Core group-state checks require admin signatures, preserve membership invariants and require the next epoch for accepted updates. Remaining: native always-on-top dock/bubbles, real compact-window resizing, persistent settings/profile, live network group/file actions, TLS calls/signaling and WebRTC. This is a foundation build; later phases are not yet complete.
