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
- [x] Settings → Opacity reaches the dock through the snapshot and fades the icon and bubbles (`e2e/dock.spec.ts`).
- [x] Settings → Idle icon, Screen corner and the tray's mute/DND labels are driven by the stored settings, with `drag::corner_position` and `platform::tray_toggle_titles` unit-tested.
- [x] The dock's state frames come from `packages/ui/src/state.ts` and `screens/dock.ts` rather than inline rules: Do Not Disturb (orange ring, minus chip, no bubbles) and In Call (green ring, phone + elapsed chip, unread badge hidden) are asserted in `e2e/dock.spec.ts` against the `dnd` and `active-call` mock scenarios.
- [x] The unread pulse is silenced under `prefers-reduced-motion: reduce`, as the board requires.
- [x] The quiet rule behind the DND ring is the rule that suppresses the alert (`platform::icon_is_quiet`, read by `should_notify`), re-checked once a second so a scheduled window opens the state on time.
- [x] Chat rows and bubbles show a real timestamp on desktop (`PeerDto.last_message_ms`); the bubble stack and the icon avatar follow recency, not alphabetical order.
- [x] Settings → Icon morph and Revert delay reach the dock through the snapshot: the sender's face is worn only while the switch is on, is dropped after the stored delay, and is never dropped during a call (`e2e/dock.spec.ts`, three scenarios).
- [x] The morphed face is drawn as the board draws it — a circle in the middle of the icon, not the unread badge's corner square — asserted from geometry (`border-radius`, `position`, centre within 3 px) rather than from class names, and a quiet device gives the bottom-right corner to the minus chip (`e2e/dock.spec.ts`).

## Manual checks still required on real systems

- [ ] Move/resize the main window; dock remains outside its bounds.
- [ ] Minimize and close the main window; dock stays visible.
- [ ] Put a browser or another app in front; dock stays above it.
- [ ] macOS: switch Spaces and enter a fullscreen app; verify overlay level and non-activation.
- [ ] Windows: verify topmost behavior over borderless-fullscreen apps. Exclusive-fullscreen games are unsupported by Windows overlay windows.
- [ ] Open Quick chats while typing in another app; verify it does not activate the main window and confirm desired keyboard focus behavior.
- [ ] Click transparent padding around dock and panel; verify underlying apps receive clicks. Test pointer transitions into and out of hit regions.
- [ ] Receive a message on a second device and watch the dock bubble. Transport, quick reply and the "Show message preview" rule are wired to the real peer session and verified between scratch peers in the main window (docs/PROGRESS.md items 6a, 8a, 10a), but the bubble has not been observed on a second machine.
- [ ] Test left/right placement, at least two monitors, hot-plugging, and 100%, 125%, 150%, and 200% display scaling.
- [ ] Drag the dock and confirm compact panel follows it.

## Current implementation limits

The menu route provides Open Quick chats, Open AlverChat, Reset icon position, and Quit. The native peer list updates from signed mDNS announcements, and received messages arrive over the encrypted peer session, which is what feeds the dock bubbles and the compact window's chats tab. On macOS the Tauri NSWindow receives the non-activating style mask, elevated level, and Space collection flags; it has not been converted to an NSPanel object. Windows-specific behavior has not been run on Windows. Native cursor polling now updates hit regions while the pointer crosses click-through windows, but its behavior still needs real-system validation.

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
- [ ] Mute from Settings (or enter the scheduled window) and confirm the icon turns orange with the minus chip, no bubble appears for the next message, and the ring starts again the moment the quiet ends — the state is re-read on the clock, not on an event.
- [ ] On a second machine, ring and answer a call and watch the dock: green ring, phone chip counting up from the answer, unread badge gone until the call ends. Hang up by closing the peer's laptop instead of the call, and confirm the icon clears when that peer drops out of the network.

Cross-platform pointer scaling and click-through re-entry still need real Windows and macOS validation. The current implementation uses WebView pointer capture and converts pointer deltas with the active window's scale factor; Windows code has not been compiled in this macOS environment.
