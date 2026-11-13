# Overlay widget verification

The overlay windows are created by Rust as `dock`, `compact`, and `menu` WebviewWindows. The UI routes use `/#/dock`, `/#/compact`, and `/#/menu`. `pnpm dev:overlay-check` starts the desktop app with the overlay windows enabled.

## Automated or scripted checks

- [x] TypeScript and Vite production build completes (`pnpm build`).
- [x] Tauri Rust build and command permissions compile (`cargo check -p alverchat-desktop`).
- [x] `dock`, `compact`, and `menu` are created independently from the main window.
- [x] Closing the main window is intercepted and hides it; the desktop app continues running.
- [x] Compact placement uses the dock monitor's work area, scale factor, available side, and clamped bounds.
- [x] Dock shrinks to an idle icon and expands to fit up to three message bubbles.
- [x] Tray menu and dock context menu include Open AlverChat and Quit actions.
- [x] Compact route listens for Escape and native focus loss; explicit close also hides it.
- [x] Overlay route requests a Rust snapshot and listens for `overlay-state` events.

## Manual checks still required on real systems

- [ ] Move/resize the main window; dock remains outside its bounds.
- [ ] Minimize and close the main window; dock stays visible.
- [ ] Put a browser or another app in front; dock stays above it.
- [ ] macOS: switch Spaces and enter a fullscreen app; verify overlay level and non-activation.
- [ ] Windows: verify topmost behavior over borderless-fullscreen apps. Exclusive-fullscreen games are unsupported by Windows overlay windows.
- [ ] Open Quick chats while typing in another app; verify it does not activate the main window and confirm desired keyboard focus behavior.
- [ ] Click transparent padding around dock and panel; verify underlying apps receive clicks. Test pointer transitions into and out of hit regions.
- [ ] Send a new message while main is hidden; verify icon/bubble updates and quick reply delivery.
- [ ] Test left/right placement, at least two monitors, hot-plugging, and 100%, 125%, 150%, and 200% display scaling.
- [ ] Drag the dock and confirm compact panel follows it.

## Current implementation limits

The menu route provides Open Quick chats, Open AlverChat, Reset icon position, and Quit. The Rust peer list is preview data; LAN discovery and live incoming-message transport are not implemented in this repository yet. On macOS the Tauri NSWindow receives the non-activating style mask, elevated level, and Space collection flags; it has not been converted to an NSPanel object. Windows-specific code has not been compiled on Windows in this environment. Native cursor polling now updates hit regions while the pointer crosses click-through windows, but its behavior still needs real-system validation.

## Draggable widget checks

### Automated

- [x] 4 px threshold, work-area clamping, nearest/near-edge snap, vertical-percentage restore, bubble edge choice, and panel follow/detach rules have Rust unit tests (`cargo test -p alverchat-desktop`).
- [x] Drag moves are batched through `requestAnimationFrame`; native position changes use monitor scale factors.
- [x] Position is stored per monitor name, edge, and vertical percentage and restored at startup; a missing monitor falls back to the primary display.
- [x] Dock and compact panel use the same pointer-capture drag hook; compact header controls/search/tabs are excluded from the drag handle.
- [x] Native click-through stays disabled from pointer-down through pointer-up/cancel, including before the 4 px drag threshold.
- [x] Reset is available in Settings, the dock context menu, and tray menu.
- [x] Snap animation observes `prefers-reduced-motion`.

### Manual checks still required

- [ ] Drag the icon over apps and between monitors at different scale settings; check that it stays under the pointer.
- [ ] Short click opens Quick chats; moving 4 px or more drags without opening it; double click has no additional action.
- [ ] Drag and release near each edge; quit/relaunch and confirm placement restore.
- [ ] Disconnect/reconfigure a monitor and confirm the dock is clamped to a visible work area.
- [ ] Drag the icon while compact is open; panel follows. Drag the panel header; panel detaches and remains clamped.
- [ ] Test with fullscreen app frontmost and confirm drag does not activate AlverChat or steal typing focus.
- [ ] Test click-through before, during, and after drag, including fast pointer exits.
- [ ] Enable Reduce Motion and confirm edge snap moves instantly.
- [ ] Use Settings, the dock context menu, and the tray menu to reset the icon position.

Cross-platform pointer scaling and click-through re-entry still need real Windows and macOS validation. The current implementation uses WebView pointer capture and converts pointer deltas with the active window's scale factor; Windows code has not been compiled in this macOS environment.
