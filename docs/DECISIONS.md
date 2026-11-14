# Decisions

- 2026-11-13: Use a Vite root with shared packages and a Tauri shell because the provided workspace was empty and the requested desktop client is Tauri-based.
- 2026-11-13: Keep the first runnable slice mock-backed so all UI states can be explored without LAN peers.
- 2026-11-13: Keep the first visible peer list and conversation data mock-only; this makes every UI state reviewable without claiming a live LAN session.
- 2026-11-13: Use source board originals plus deterministic 8-cell crop sheets and pixel palette samples under design/frames and docs/design-colors.json for review.
- 2026-11-13: Keep group creation in the mock UI session state until signed group-state persistence and LAN fan-out are implemented; preserve the selected avatar palette color in the local group view.
- 2026-11-13: Sign group rosters over a fixed-order JSON tuple with sorted, unique member IDs so every peer can verify admin authority and detect stale epochs consistently.

## Overlay widget audit — 2026-11-13

1. The floating icon is `.floating-dock` in `App.tsx`, fixed to the main app viewport.
2. The unread stack is `.dock-bubbles` in the same main React tree.
3. The Quick chats drawer is `.compact-drawer`, also fixed to that viewport.
4. Those nodes inherit the main window's clipping, visibility, focus, and coordinate space.
5. `tauri.conf.json` currently declares only the `main` window.
6. `main.rs` currently has no `WebviewWindowBuilder` calls and no overlay lifecycle commands.
7. No source capabilities file exists; only generated Tauri schema files are present.
8. Decision: create `dock`, `compact`, and `menu` as Rust-owned windows with hash routes in the shared UI bundle.
9. Decision: keep overlay data in Rust-managed state and publish snapshot/update events; do not bridge state through localStorage.
10. Decision: use Tauri's native always-on-top, transparency, workspace, drag, and cursor-ignore APIs; record platform tests that need real OS validation.

## Draggable overlay widgets — 2026-11-13

- Create `feature/draggable-widget` from the overlay branch and keep each change in a focused commit.
- Use one React pointer-capture hook for dock and compact-panel dragging, with a 4 px threshold and animation-frame delta batching.
- Move native windows with Rust `set_position` deltas so the dock remains non-activating and clicks are handled separately from drags.
- Keep compact attached while the dock moves; dragging its header marks it detached, and reopening resets it to the dock anchor.
- Save dock monitor name, edge, and vertical percentage in the app configuration directory; restore to the saved monitor when available and primary display otherwise.
- Snap on release when edge snapping is enabled or the icon is within 24 logical pixels; honor system reduced-motion preference.
- Use Rust geometry helpers with unit coverage for drag threshold, clamping, snapping, percentage restore, bubble side, and panel attachment.

## Phase 5 shell and screens — 2026-11-13 19:57

- Continue on `main` instead of creating `build/v1`, because the published repository history and the pushed checkpoint tags already live there; creating the branch mid-build would fork the release lineage for no benefit.
- Resample only `--danger` (to #eb453d) and leave `--text` and `--accent2` at their existing values: the cross-board spread for those two tokens is 16.7 and 15.4, and the boards disagree per-screen rather than showing one intended value.
- The onboarding gate is one-time per bridge load, keyed on `identity.peerId` being absent, so a peer with an empty display name is not forced back into onboarding on every start.
- `ChatList.onOpenNetwork` lands on the Settings view generally, not on its network category, because `SettingsScreen` owns the category state internally and exposing it would widen the screen contract for one link.
- Reply quoting is kept in the mock bridge and dropped by the desktop shell: `messages` has no `reply_to` column, and Tauri v2 ignores the extra `replyToId` argument, so the UI stays honest rather than pretending persistence. Silently continuing to quote in mock is intentional so the design state remains reviewable. Until migration v5 lands.
- Narrow windows use the `mobile-chat` class to swap list and conversation panes; the designs only show the three-pane desktop layout, so this is the closest reading of the existing `.app` grid.
- Every call CTA toasts "unavailable until LAN call signaling is connected" while `listCalls` history renders for real, so the Calls screen shows what exists without advertising what does not.
- The shell refreshes a thread through a `revision` counter bumped by `messages-changed`, with a `seenRevision` guard plus `key={chat.id}`, because a per-chat event subscription would re-subscribe on every chat switch.

## Phase 3/4 commands, typing and read state — 2026-11-13 20:55

- Reply quoting became an `Option<Uuid>` side-channel (`enqueue_quoted_message`, `set_reply_to`, `reply_to_for`, `replies_for_chat`) plus migration v5, instead of a `reply_to` field on `Message`: the field would have touched ~17 construction sites and broken the property the migration tests guard, that v1 rows are still written with v1-shaped SQL.
- Read receipts (`kind = 'read'`, reader `local`) are the unread model, so "Mark unread" is durable and `mark_as_read` finally writes state. The conversation reads the unread span from `listMessages` *before* it clears it — `mark_unread` is a mutation and must not be used as a query.
- `MessageDto.read` is only true for messages I sent (no peer read receipts exist yet) and for received messages the store has a read receipt for; the UI divider is derived from that field rather than from a second round-trip.
- Typing became a second accepted envelope kind inside the existing peer session, answered with the same receipt frame and expired after 6 s by the discovery tick, rather than a new connection type or a timer thread: it reuses the authenticated session, and the expiry makes a lost `false` frame self-healing.
- `deliver_envelope` was split into `send_frame` (handshake + one frame + receipt) plus the outbox acknowledgement, because typing needed the transport without the outbox.
- Group descriptions, icons and the "left" flag live in the `settings` table keyed `group.description.<id>` / `group.icon.<id>` / `group.left.<id>`, because the signed `GroupState` format is frozen and only name and roster are covered by its signature.
- `pinned_by` in `pinned_messages` is always this device: pins are local until a `pin` frame exists, and claiming a peer pinned it would be a lie in the UI.
- `leave_group` only records the local `left` flag. Removing myself from the roster needs the admin's signature, and `GroupState::update_members` refuses with `CannotRemoveAdmin` for the admin themself.
- Commands, DTOs and the `GroupPatch` input are `pub(crate)` so `generate_handler!` can name them from the crate root, and the store-lock helper uses `State::inner()` to keep the guard borrowed from the app instead of from a temporary.
- Outgoing messages keep the literal sender id `local` while group and roster authority uses `identity.peer_id()`: switching the message actor would silently orphan existing history for `mine`, edit and tombstone checks.

## LAN announcements UI (spec 7.12) — 2026-11-13 21:10

- The full-width banner lists only announcements that still owe *this device* a tap. My own posts stay out of it and keep their "Acknowledged by (N)" trail in Group Info, because a persistent strip over every screen for a notice I wrote myself would be noise, and the board shows the trail as a sender-side detail under the banner.
- `send_announcement` now reads `announcementsFromAdminsOnly` from the live `SettingsState` instead of the `announcement.admins_only` store key: settings are persisted to JSON by `platform::persist`, so the DB key could never have been true, and the admins-only toggle in Privacy & Network would have done nothing.
- Group Info posts and acknowledges through the bridge and then calls a new optional `ScreenProps.refreshAnnouncements`, rather than introducing a shared store (zustand is a dependency but the UI has stayed prop-driven): one callback keeps the shell banner and the panel honest without a new pattern.
- `AnnouncementsSection` reuses the design-board `AnnouncementBanner` instead of a new component, and the banner gained an optional `meta` slot plus "no action when `acknowledge` is absent", so an author's own row shows no dead Acknowledge button.
- `packages/ui/src/announcements.ts` owns the id↔name and ms↔stamp conversions, and treats a rejected `listAnnouncements` as an empty group: the bridge has no all-groups call, so the banner is assembled from `listGroups` fan-out and one refusing group must not hide the rest.
- Announcements are still local-only over the wire — no `announcement` / `announcement_ack` frame is sent, so a peer never sees this device's posts and remote acks never arrive. Same gap as group fan-out, tracked in BLOCKERS.

## Chunked file transfer (receiver pulls) — 2026-11-14 01:06

- The receiver pulls: it opens the session, sends `file_accept{resume_from}`, and only then does the sender stream, over that same connection. A pause, crash or reconnect costs nothing because the side that owns the part file also owns resume, and a sender never queues bytes nobody is reading.
- The transfer id *is* the attachment id *is* the message id, so a file bubble resolves its card through the primary key and no migration or new index was needed. Direction is derived from the message's sender (`local`), not stored on the attachment.
- Transfer frames are matched by `payload.transfer_id`, never by the envelope id. The core stamps a fresh id on every accept, chunk, ack and cancel, so only an offer — which the app builds itself — carries the transfer id in the envelope; comparing the two rejects every real peer.
- The database row is the user's intent and the in-memory `LiveTransfer` map is the driver's progress. A driver reads the row between batches and never writes it mid-run, or a `transferring` write would undo a pause the user clicked a moment earlier.
- Only `Transferring` keeps live progress. `Failed` is not terminal in the core state machine, so a stale entry would survive a failure; the map entry is dropped for every other state and a stopped download reports from its resume index instead.
- Progress reaches the UI through the existing `messages-changed` event, at most twice a second. A 500 MB file crosses thousands of batch boundaries and the shell already refetches the thread on that signal, so a new per-tick event would only add vocabulary and load.
- `push_accepted` and `SendRequest` exist because the desktop's session loop has already consumed the `file_accept` frame by the time it decides to stream; reading the accept twice would deadlock the session. The core keeps `push_chunks` as the one-call form for tests and other callers.
- Size policy comes from `platform::SettingsState::snapshot()` (settings.json), not the store's settings rows — those rows are never written, the same reason `send_announcement` reads the live settings. An empty `max_file_size_mb` falls back to `files::DEFAULT_MAX_FILE_BYTES` rather than rejecting every file.
- A re-offer of a known transfer resumes an `Accepted`, `Failed` or stale `Transferring` download, but leaves `Paused` and `Offered` alone: the sender's retry heals a broken pull without overriding the user's pause or a download nobody asked for.
- Pausing or cancelling a download that no driver is watching sends one advisory `file_cancel` on a short-lived session and ignores the failure. The peer's session loop deliberately sends no receipt for a cancel, and a peer that walked away must not turn a local button press into an error.
- Outgoing bytes stay in `<app-data>/outgoing`, incoming files land in the user's `downloads_dir` (default `<app-data>/downloads`), and the resume sidecar sits beside the part file so a moved folder carries the file and its resume state together. On-disk names come from `stored_name(id, file_name)`, so a peer cannot choose a path or overwrite another transfer.
- `send_file` takes the file as a base64 data URL and `open_attachment` hands the resolved path to `tauri-plugin-opener`: the picker keeps ownership of file access in the webview, and the shell never invents a filesystem handle.

## Developer secrets without the keychain — 2026-11-14 01:20

- Secrets go through one `secrets::Secrets` store instead of three `keyring::Entry` calls, so the database key, the identity and the LAN passphrase cannot drift into different storage rules.
- `ALVERCHAT_DEV_SECRETS` swaps the keychain for `0600` files under the instance data directory, but only in debug builds (`cfg!(debug_assertions)`): the macOS prompt is really "this ad-hoc-signed binary is not the one that created the item", and a shipped binary must never be talkable into keeping keys in a file.
- A profile is refused when its store does not match its database, in both directions, with the fix in the message. Without it the first hatch run fails as "file is not a database" after quietly generating a key, and unsetting the flag afterwards would leave the profile unopenable.
- The hatch is documented with a scratch `ALVERCHAT_DATA_DIR`, because reusing the real profile is exactly the mismatch the guard refuses; two dev profiles also stop sharing one identity.

## Message latency — 2026-11-14 12:00

- macOS resolves a peer's mDNS name to its link-local IPv6 address before the IPv4 one, and a connect to that address stalls for the whole five-second timeout before IPv4 is tried. `connect_to_peer` now races every resolved address and keeps the first that connects, so a send is bounded by the reachable address instead of the unreachable one.
- One dialed session per peer stays warm for 30 seconds (`OUTBOUND_IDLE`), so a message, typing flag or receipt reuses it instead of paying another resolve, TCP connect and TLS handshake. The responder closes a silent session after 60 seconds, so the window stays inside that. Any send error drops the session and retries once with a fresh dial; a replayed envelope is harmless because the store dedupes by message id.
- The composer queues the message and returns: the bubble appears immediately as pending, and the outbox plus its retry loop own delivery. The status flips to sent through the existing `messages-changed` signal when the receipt lands, so the UI never blocks on a peer handshake.



## File offers reach the receiver — 2026-11-14 12:20

- An incoming `file_offer` was compared against the sender's peer id (`payload.chat_id != remote`), but the app builds `chat_id` as the *receiver* it addresses. Every offer was therefore rejected as "not theirs", the receiver closed the session without a receipt, and the sender retried forever with a bubble stuck at "Sending… 0%". The check now compares the payload's `chat_id` with this device's own peer id.
- The outbox retry loop used to exit whenever the peer had no endpoint, so an offer queued while discovery was between resolutions was never delivered. It now backs off (1 s → 60 s) and keeps watching the pending queue instead of breaking.
- A re-offer with no current endpoint no longer drops its delivery attempt: the retry loop owns the queued offer and delivers it when the peer reappears.
- Verified with two fresh profiles on one machine: two 615 KB offers were auto-accepted, pulled, blake3-verified and renamed into the receiver's downloads folder; both databases report `Completed` and both outboxes are empty.

## Phase 6 calls — 2026-11-14

- The Rust shell stays a signaling relay, not a call state machine: `send_call_signal` turns one webview signal into the protocol frame, enqueues it on the durable outbox, and `handle_signal` records the call log and re-emits the frame to the UI as `call-signal`. SDP/ICE bodies are opaque to Rust; media never leaves the WebRTC stack.
- Call frames ride the existing warm outbound session beside messages (whitelist extended by `calls::handled_kind`), so signaling inherits TLS, peer-key pinning, receipts and offline outbox retry for free. A missing endpoint queues the frame instead of dropping it — signaling may be the first frame after a reconnect.
- One live call at a time, and the caller is the only side that renegotiates (screen share, camera on/off): the receiver answers every offer. That avoids the perfect-negotiation glare machinery for a two-peer call.
- WebRTC runs with `iceServers: []` — host candidates only, LAN by construction, nothing leaks to a STUN server.
- Both the full app and the compact quick window mount their own `useCall` and listen to `call-signal`, so an incoming call rings in whichever surface is open; the call surface sits in an `overlay-hit` wrapper so the click-through hit regions keep working over the compact window.
