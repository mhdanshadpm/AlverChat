# Build progress

Updated 2026-11-13 19:57 (Asia/Kolkata).

After a context reset, read this file first and resume at the first unchecked item.

## Phases

- [x] Phase 1 — Rust workspace, typed bridge, UI foundation, empty browser adapter, design assets, CI baseline. (2026-11-13)
- [ ] Phase 2 — Ed25519 identity + signed mDNS discovery wired into the desktop app; identity and SQLCipher keys use the OS credential store; interim Noise XX/TCP messaging, pinned peer keys, offline outbox retry, delivery acknowledgement, received-message persistence. Remaining: required TLS 1.3 `tokio-tungstenite`, heartbeat presence, cross-device verification, LAN passphrase challenge enforced in the transport.
- [ ] Phase 3 — Encrypted local history + persistent outbox; demo conversations removed; direct exchange and receipts over the interim transport. Remaining: group fan-out, edit/reaction synchronisation, file transfer.
- [x] Phase 4 — Native dock and compact windows: draggable, saved position, edge snapping, click-through hit testing, bubble timeout, click-to-open. Needs hands-on Windows/macOS validation.
- [ ] Phase 5 — Screens, profile editing, settings UI. See screen checklist below.
- [ ] Phase 6 — Calls: layouts, signaling, WebRTC, group calls, PiP, screen sharing. Not connected.
- [ ] Phase 7 — Updater configured for signed GitHub Release artifacts, check/install flow, release workflow (Windows x64 + Apple Silicon). Remaining: backup/export/restore, tray extras, hotkeys, i18n registration.
- [ ] Phase 8 — Rust/UI CI checks pass locally. Remaining: signing/notarization, security fuzzing, performance, accessibility, stray-URL grep allowlist.
- [ ] Phase 9 — Linux/mobile stubs, legacy importer (no `/legacy` folder exists in this repo).

## Screens (Phase 5)

| Screen | UI | Desktop data path | Mock path | Status |
| --- | --- | --- | --- | --- |
| Chat list | done | `list_peers`, `mark_as_read`, `set_pinned_chat`, `set_muted_chat` | done | wired |
| Conversation | done | `list_messages`, `send_message`, `edit_message_local`, `delete_message_local` | done | partial — no `reply_to` column, no reactions/pins/stars/search/attachments commands in Rust yet |
| Chat details | done | none yet | done | UI only |
| Groups | done | missing (`list_groups` et al.) | done | blocked on Rust commands |
| People | done | `list_peers` | done | wired |
| Calls | done | missing (`list_calls`) | done | blocked on Rust commands |
| Settings | done | `get_settings`, `update_settings`, `set_hotkey`, `set_lan_passphrase`, `permission_status`, `run_connection_test`, backup stubs | done | partial |
| Profile | done | `get_my_identity`, `get_avatar`/`set_avatar` (missing) | done | partial |
| Onboarding | done | identity + settings | done | wired |
| Component gallery | done | n/a | n/a | wired |
| Overlay dock / compact / menu | done | overlay commands | n/a | built, needs OS validation |

## Immediate queue

1. [x] App shell: rail, view router, Tauri event subscriptions, toast viewport, narrow-window chat pane. (2026-11-13 19:57, committed)
2. [ ] Migration v5: `messages.reply_to` + persist `sendMessage(chatId, text, replyToId)`.
3. [ ] `apps/desktop/src-tauri/src/commands.rs`: register the 29 missing bridge commands over the existing `alver-core` store APIs.
4. [ ] Wire the new commands into `Conversation`, `GroupsScreen`, `CallsScreen`, `ChatDetails`, `ProfileScreen`.
5. [ ] Phase 6 calls (WebRTC + signaling), Phase 7 i18n registration + docs, Phase 8 verification suite.

## Gates at the last recorded commit

`pnpm exec tsc -b --force` clean · `pnpm exec eslint .` clean · `pnpm test` 80 passed · `pnpm build` ok · `cargo check --workspace --all-targets` clean · `cargo test --workspace` all pass.

Public repository: https://github.com/mhdanshadpm/AlverChat. The app is not production-ready until the remaining network, messaging, and platform work is completed.
