import { invoke } from '@tauri-apps/api/core';
import type { ChatMessage, OverlaySnapshot, Peer, PlatformBridge } from './index';

/** Tauri implementation. All native calls stay inside the bridge package. */
export const tauriBridge: PlatformBridge = {
  listPeers: () => invoke<Peer[]>('list_peers'),
  listMessages: (chatId) => invoke<ChatMessage[]>('list_messages', { chatId }),
  sendMessage: (chatId, text) => invoke<ChatMessage>('send_message', { chatId, text }),
  setTheme: (theme) => invoke<void>('set_theme', { theme }),
  overlaySnapshot: () => invoke<OverlaySnapshot>('overlay_snapshot'),
  toggleCompact: () => invoke<void>('toggle_compact'),
  hideCompact: () => invoke<void>('hide_compact'),
  openFullApp: (chatId) => invoke<void>('open_full_app', { chatId }),
  setOverlayHitRegions: (hasHit) => invoke<void>('set_overlay_hit_regions', { hasHit }),
  startOverlayDrag: () => invoke<void>('start_overlay_drag'),
  resizeDock: (hasBubbles) => invoke<void>('resize_dock', { hasBubbles }),
  openDockMenu: () => invoke<void>('open_dock_menu'),
  hideMenu: () => invoke<void>('hide_menu'),
  quit: () => invoke<void>('quit_app'),
};
