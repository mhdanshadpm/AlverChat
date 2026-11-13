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
