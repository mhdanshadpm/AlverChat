# Blockers and remaining gaps

- 2026-11-13: Signed mDNS discovery and the required TLS 1.3 `tokio-tungstenite` authenticated Noise XX transport are implemented with Ed25519 identity verification, peer-key pinning, bounded encrypted frames, a persistent encrypted outbox, delivery acknowledgements, retry, and received-message persistence. Local unit tests and verification pass.
- The existing Tauri overlay build passed locally on macOS, but native window behavior has not been visually verified in this session. CUA reported that no desktop windows were available. Verify dragging, click-through, monitor positioning, startup, keychain access, and local-network permissions on macOS and Windows.
- The public GitHub repository and CI/release workflows exist. Run 43389740873 passed UI and macOS; Windows Clippy is finishing. Run 43390639346 checks the latest desktop-script update. CI builds SQLCipher with vendored OpenSSL to avoid runner-specific OpenSSL configuration.
- Calls/WebRTC, group fan-out and membership UI, file transfer, profile/avatar network sync, database backup/export/restore, signing/notarization, and most settings integrations are not implemented. Their UI is disabled or shows unavailable states where reviewed; do not advertise these as working.

## Phase 5 shell and screens — 2026-11-13 19:57

- 29 bridge methods have no Tauri command behind them (`list_groups`, `create_group`, `update_group`, `add_group_members`, `remove_group_member`, `leave_group`, `set_avatar`, `get_avatar`, `pin_message`, `pinned_messages`, `star_message`, `starred_messages`, `reactions_for`, `react`, `forward_messages`, `search_messages`, `set_typing`, `mark_unread`, `clear_chat`, `delete_chat`, `attachments_for`, `send_file`, `control_transfer`, `open_attachment`, `list_announcements`, `send_announcement`, `acknowledge_announcement`, `list_calls`, `delete_for_everyone`). They reject in the desktop app; the persistence primitives already exist in `crates/alver-core`, so this is glue work, not a design blocker.
- Reply threading cannot be persisted until a `messages.reply_to` migration exists.
- Three subagents hit the 150-turn ceiling before reporting; the work was finished in the main session. Risk note for future phases: delegate narrow tasks, not whole phases.
- Playwright is not installed, so Phase 8 screenshots are blocked; use `?mock=1&scenario=…` in a normal browser for visual review until then.
- The LAN passphrase gates the settings file but the transport does not enforce an HMAC challenge yet.
- Native overlay windows compile and the commands are registered, but were not visually validated on real macOS or Windows in this session.
