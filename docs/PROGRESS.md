# Build progress

Updated 2026-11-14 01:06 (Asia/Kolkata).

After a context reset, read this file first and resume at the first unchecked item.

## Phases

- [x] Phase 1 — Rust workspace, typed bridge, UI foundation, empty browser adapter, design assets, CI baseline. (2026-11-13)
- [ ] Phase 2 — Ed25519 identity + signed mDNS discovery in the desktop app; identity and SQLCipher keys in the OS credential store; Noise XX over TCP, pinned peer keys, offline outbox retry, delivery acknowledgement, received-message persistence, typing frames. Remaining: required TLS 1.3 `tokio-tungstenite`, heartbeat presence, cross-device verification, LAN passphrase challenge enforced in the transport.
- [x] Phase 3 — Encrypted local history + persistent outbox; direct exchange and receipts; reactions, pins, stars, edit/delete/forward, reply quoting (`messages.reply_to`), unread/read state, FTS search, group roster commands, announcement storage, call history, avatar cache. Remaining in later phases: group message fan-out, WebRTC signaling.
- [x] Phase 4 — Native dock and compact windows: draggable, saved position, edge snapping, click-through hit testing, bubble timeout, click-to-open. Needs hands-on Windows/macOS validation.
- [ ] Phase 5 — Screens. Every screen is built and now calls real commands; still open: the announcement bubble beside the dock icon, call UI (7.9), and OS validation of the overlays.
- [x] Phase 6 core — 1:1 LAN calls: signaling over the encrypted peer session, WebRTC voice/video in the webview, ringtone, mute/camera/screen-share, PiP, device picker, call log entries from live calls. (2026-11-14) Remaining: group calls, glare-free renegotiation, OS validation on Windows.
- [ ] Phase 7 — Updater configured for signed GitHub Release artifacts, check/install flow, release workflow (Windows x64 + Apple Silicon). Remaining: backup/export/restore verification, tray extras, hotkeys, i18n registration.
- [ ] Phase 8 — Rust/UI CI checks pass locally. Remaining: signing/notarization, security fuzzing, performance, accessibility, screenshot comparison, stray-URL grep allowlist.
- [ ] Phase 9 — Linux/mobile stubs, legacy importer (no `/legacy` folder exists in this repo).

## Screens (Phase 5)

| Screen | UI | Desktop data path | Mock path | Status |
| --- | --- | --- | --- | --- |
| Chat list | done | `list_peers`, `mark_as_read`, `mark_unread`, `set_pinned_chat`, `set_muted_chat`, `clear_chat`, `delete_chat` | done | wired |
| Conversation | done | thread, send with quote, edit, delete, reactions, pins, stars, forward, search-in-chat, attachments list, typing in and out, chunked file send/receive with pause, resume, cancel and open | done | wired — read receipts from peers still missing |
| Chat details | done | `pinned_messages`, `starred_messages`, `attachments_for`, `open_attachment`, `list_calls` | done | wired |
| Groups | done | `list_groups`, `create_group`, `update_group`, `add_group_members`, `remove_group_member`, `leave_group` | done | wired — announcement frames are not sent yet, so posts stay on this device (7.12) |
| People | done | `list_peers`, `get_avatar` | done | wired |
| Calls | done | `list_calls`, `send_call_signal` relay + live signaling frames, WebRTC media | done | wired — 1:1 voice/video |
| Settings | done | `get_settings`, `update_settings`, `set_hotkey`, `set_lan_passphrase`, `permission_status`, `run_connection_test`, backup stubs | done | partial |
| Profile | done | `get_my_identity`, `get_avatar`/`set_avatar` (data URL → 256 px, EXIF-free cache) | done | wired |
| Onboarding | done | identity + settings | done | wired |
| Component gallery | done | n/a | n/a | wired |
| Overlay dock / compact / menu | done | overlay commands | done | built, needs OS validation — compact is now a full quick-chat shell (tabs, in-place conversations) |

## Immediate queue

1. [x] App shell: rail, view router, Tauri event subscriptions, toast viewport, narrow-window chat pane. (2026-11-13 19:57, committed)
2. [x] Migration v5 `messages.reply_to` + `sendMessage(chatId, text, replyToId)` persisted both ways. (2026-11-13 20:31, committed)
3. [x] `apps/desktop/src-tauri/src/commands.rs`: the 29 missing bridge commands registered over `alver-core`. (2026-11-13 20:31, committed)
4. [x] Typing frames over the wire + read receipts as the unread source (`unread_ids_for_chat`, `MessageDto.read`, `markAsRead`). (2026-11-13 20:52, committed)
5. [x] LAN announcements UI: composer in Group Info, full-width in-app banner with Acknowledge, "Acknowledged by (N)" trail, admins-only setting honoured. (2026-11-13 21:10)
6. [x] File chunks: receiver-pulled chunk streaming over the authenticated session, blake3-verified, resumable from a sidecar index, pause/resume/retry/cancel, downloads folder + per-file progress in the bubble. `send_file` / `control_transfer` / `open_attachment` move real bytes. (2026-11-14 01:06)
6a. [x] File offers actually arrive: the receiver compared the offer's `chat_id` (which names the receiver) with the sender id and rejected every offer; the outbox retry loop also exited when a peer briefly had no endpoint. Fixed both and verified two 615 KB transfers end-to-end between fresh profiles. (2026-11-14 12:20)
6b. [x] Compact quick-chat window rebuilt as a mini app: functional Chats/Groups/People/Calls tabs reusing the real screens, chats open as an in-place slide-in conversation, group chats merged into the list with preview/unread, theme follows settings, onboarding gate points at the full app. (2026-11-14 12:15)
7. [x] Phase 6 calls (1:1): `calls.rs` relays call_offer/answer/ice/end/reject/busy over the peer session and records the call log; `useCall` drives WebRTC (host candidates only, no STUN) with the caller-only renegotiation rule; `CallOverlay` renders incoming/ringing/active/ended/pip; wired into the full app and the compact quick window, which answers its own copy of every frame. Group calls stay out. (2026-11-14)
7a. [x] macOS media fix: WKWebView silently denied `getUserMedia` on signed builds because no entitlements declared the audio/camera capabilities — added `apps/desktop/src-tauri/entitlements.plist` (audio-input, camera, hardened-runtime CS flags) and pointed `bundle.macOS.entitlements` at it. Live two-instance check on the same machine: peers discover each other via mDNS (requires `ALVERCHAT_MOCK_PEER` so each instance gets its own identity/keychain entry), Voice call reaches the "Calling…" overlay with mute/screen-share/PiP/controls, and hang-up renders the Ended card. Full offer/answer/active-media verification still pending a stable UI-driving session. (2026-11-14)
8. [ ] Phase 7: i18n catalog registration, README firewall ports, updater/tray verification.
9. [ ] Phase 8: vitest units for the bubble stack, icon state machine, DND and menu placement; screenshots of every screen in mock light+dark into `/design/_compare/`; CI stray-URL grep.

## Gates at the last recorded commit

`pnpm exec tsc -b` clean · `pnpm exec eslint .` clean · `pnpm test` 100 passed · `pnpm build` ok · `cargo fmt --all -- --check` clean · `cargo test --workspace` 98 + 9 + 5 + 13 + 15 passed · `cargo clippy --workspace --all-targets` clean.

Public repository: https://github.com/mhdanshadpm/AlverChat. The app is not production-ready until the remaining network, messaging, and platform work is completed.
