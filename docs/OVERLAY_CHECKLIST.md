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

Tauri's WebView cursor-ignore switch is driven by pointer events inside each window. Native behavior must be verified because pointer re-entry after click-through can vary by OS/window manager. The `menu` route currently provides Open Quick chats, Open AlverChat, and Quit. The Rust peer list is preview data; LAN discovery, live incoming-message transport, and per-monitor position persistence are not implemented in this repository yet. On macOS the Tauri NSWindow receives the non-activating style mask, elevated level, and Space collection flags; it has not been converted to an NSPanel object. Windows-specific code has not been compiled on Windows in this environment. Hit-region changes rely on WebView pointer events, so pointer re-entry after click-through needs real-system validation and may require native cursor polling.
