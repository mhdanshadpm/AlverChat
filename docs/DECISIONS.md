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
