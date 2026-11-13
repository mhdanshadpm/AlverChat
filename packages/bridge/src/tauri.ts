import { invoke } from '@tauri-apps/api/core';
import type { ChatMessage, OverlaySnapshot, Peer, PlatformBridge } from './index';

/** Tauri implementation. All native calls stay inside the bridge package. */
export const tauriBridge: PlatformBridge = {
  listPeers: () => invoke<Peer[]>('list_peers'),
  listMessages: (chatId) => invoke<ChatMessage[]>('list_messages', { chatId }),
  sendMessage: (chatId, text) => invoke<ChatMessage>('send_message', { chatId, text }),
  overlaySnapshot: () => invoke<OverlaySnapshot>('overlay_snapshot'),
  toggleCompact: () => invoke<void>('toggle_compact'),
  hideCompact: () => invoke<void>('hide_compact'),
  openFullApp: (chatId) => invoke<void>('open_full_app', { chatId }),
  setOverlayHitRegions: (hasHit) => invoke<void>('set_overlay_hit_regions', { hasHit }),
  setOverlayPointerCapture: (active, hasHit) => invoke<void>('set_overlay_pointer_capture', { active, hasHit }),
  startOverlayDrag: () => invoke<void>('start_overlay_drag'),
  moveOverlayBy: (deltaX, deltaY) => invoke<void>('move_overlay_by', { deltaX, deltaY }),
  endOverlayDrag: (hasHit, reduceMotion) => invoke<void>('end_overlay_drag', { hasHit, reduceMotion }),
  resizeDock: (hasBubbles) => invoke<void>('resize_dock', { hasBubbles }),
  openDockMenu: () => invoke<void>('open_dock_menu'),
  hideMenu: () => invoke<void>('hide_menu'),
  quit: () => invoke<void>('quit_app'),
  resetOverlayPosition: () => invoke<void>('reset_overlay_position'),
  setSnapToEdge: (enabled) => invoke<void>('set_snap_to_edge', { enabled }),
  setBubbleSettings: (enabled, autoHideSeconds) => invoke<void>('set_bubble_settings', { enabled, autoHideSeconds }),
  markAsRead: (chatId) => invoke<void>('mark_as_read', { chatId }),
  deleteMessageLocal: (messageId) => invoke<boolean>('delete_message_local', { messageId }),
  editMessageLocal: (messageId, text) => invoke<boolean>('edit_message_local', { messageId, text }),
  getMyIdentity: () => invoke<{ peerId: string; fingerprint: string }>('get_my_identity'),
};
