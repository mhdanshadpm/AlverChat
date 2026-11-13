# Build progress

Updated 2026-11-13 20:55 (Asia/Kolkata).

After a context reset, read this file first and resume at the first unchecked item.

## Phases

- [x] Phase 1 — Rust workspace, typed bridge, UI foundation, empty browser adapter, design assets, CI baseline. (2026-11-13)
- [ ] Phase 2 — Ed25519 identity + signed mDNS discovery in the desktop app; identity and SQLCipher keys in the OS credential store; Noise XX over TCP, pinned peer keys, offline outbox retry, delivery acknowledgement, received-message persistence, typing frames. Remaining: required TLS 1.3 `tokio-tungstenite`, heartbeat presence, cross-device verification, LAN passphrase challenge enforced in the transport.
- [x] Phase 3 — Encrypted local history + persistent outbox; direct exchange and receipts; reactions, pins, stars, edit/delete/forward, reply quoting (`messages.reply_to`), unread/read state, FTS search, group roster commands, announcement storage, call history, avatar cache. Remaining in later phases: group message fan-out, file chunk transfer, WebRTC signaling.
- [x] Phase 4 — Native dock and compact windows: draggable, saved position, edge snapping, click-through hit testing, bubble timeout, click-to-open. Needs hands-on Windows/macOS validation.
- [ ] Phase 5 — Screens. Every screen is built and now calls real commands; still open: LAN announcement UI (7.12), call UI (7.9), and OS validation of the overlays.
- [ ] Phase 6 — Calls: layouts, signaling, WebRTC, group calls, PiP, screen sharing. Call *history* is real (`list_calls`); signaling is not connected.
- [ ] Phase 7 — Updater configured for signed GitHub Release artifacts, check/install flow, release workflow (Windows x64 + Apple Silicon). Remaining: backup/export/restore verification, tray extras, hotkeys, i18n registration.
- [ ] Phase 8 — Rust/UI CI checks pass locally. Remaining: signing/notarization, security fuzzing, performance, accessibility, screenshot comparison, stray-URL grep allowlist.
- [ ] Phase 9 — Linux/mobile stubs, legacy importer (no `/legacy` folder exists in this repo).

## Screens (Phase 5)

| Screen | UI | Desktop data path | Mock path | Status |
| --- | --- | --- | --- | --- |
| Chat list | done | `list_peers`, `mark_as_read`, `mark_unread`, `set_pinned_chat`, `set_muted_chat`, `clear_chat`, `delete_chat` | done | wired |
| Conversation | done | thread, send with quote, edit, delete, reactions, pins, stars, forward, search-in-chat, attachments list, typing in and out | done | wired — file *chunks* and read receipts from peers still missing |
| Chat details | done | `pinned_messages`, `starred_messages`, `attachments_for`, `open_attachment`, `list_calls` | done | wired |
| Groups | done | `list_groups`, `create_group`, `update_group`, `add_group_members`, `remove_group_member`, `leave_group` | done | wired — announcements UI not built (7.12) |
| People | done | `list_peers`, `get_avatar` | done | wired |
| Calls | done | `list_calls` history only | done | partial — no signaling |
| Settings | done | `get_settings`, `update_settings`, `set_hotkey`, `set_lan_passphrase`, `permission_status`, `run_connection_test`, backup stubs | done | partial |
| Profile | done | `get_my_identity`, `get_avatar`/`set_avatar` (data URL → 256 px, EXIF-free cache) | done | wired |
| Onboarding | done | identity + settings | done | wired |
| Component gallery | done | n/a | n/a | wired |
| Overlay dock / compact / menu | done | overlay commands | n/a | built, needs OS validation |

## Immediate queue

1. [x] App shell: rail, view router, Tauri event subscriptions, toast viewport, narrow-window chat pane. (2026-11-13 19:57, committed)
2. [x] Migration v5 `messages.reply_to` + `sendMessage(chatId, text, replyToId)` persisted both ways. (2026-11-13 20:31, committed)
3. [x] `apps/desktop/src-tauri/src/commands.rs`: the 29 missing bridge commands registered over `alver-core`. (2026-11-13 20:31, committed)
4. [x] Typing frames over the wire + read receipts as the unread source (`unread_ids_for_chat`, `MessageDto.read`, `markAsRead`). (2026-11-13 20:52, committed)
5. [ ] LAN announcements UI: composer in Group Info, in-app banner with Acknowledge, "Acknowledged by (N)" trail, admins-only setting (`list_announcements`, `send_announcement`, `acknowledge_announcement` are already callable).
6. [ ] File chunks: `send_file` / `control_transfer` / `open_attachment` currently store metadata and refuse to move bytes.
7. [ ] Phase 6 calls: `MediaEngine`, signaling over the peer session, call windows, PiP, screen share, device settings.
8. [ ] Phase 7: i18n catalog registration, README firewall ports, updater/tray verification.
9. [ ] Phase 8: vitest units for the bubble stack, icon state machine, DND and menu placement; screenshots of every screen in mock light+dark into `/design/_compare/`; CI stray-URL grep.

## Gates at the last recorded commit

`pnpm exec tsc -b` clean · `pnpm exec eslint .` clean · `pnpm test` 80 passed · `pnpm build` ok · `cargo fmt --all -- --check` clean · `cargo test --workspace` 96 + 5 + 13 + 15 passed · `cargo clippy -p alverchat-desktop -- -D warnings` clean (the core crate is also lint-clean now).

Public repository: https://github.com/mhdanshadpm/AlverChat. The app is not production-ready until the remaining network, messaging, and platform work is completed.
