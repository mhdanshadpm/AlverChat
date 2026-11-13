# Decisions

- 2026-11-13: Use a Vite root with shared packages and a Tauri shell because the provided workspace was empty and the requested desktop client is Tauri-based.
- 2026-11-13: Keep the first runnable slice mock-backed so all UI states can be explored without LAN peers.
- 2026-11-13: Keep the first visible peer list and conversation data mock-only; this makes every UI state reviewable without claiming a live LAN session.
- 2026-11-13: Use source board originals plus deterministic 8-cell crop sheets and pixel palette samples under design/frames and docs/design-colors.json for review.
- 2026-11-13: Keep group creation in the mock UI session state until signed group-state persistence and LAN fan-out are implemented; preserve the selected avatar palette color in the local group view.
