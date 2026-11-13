import { invoke } from '@tauri-apps/api/core';
import type { ChatMessage, Peer, PlatformBridge } from './index';

/** Tauri implementation. All native calls stay inside the bridge package. */
export const tauriBridge: PlatformBridge = {
  listPeers: () => invoke<Peer[]>('list_peers'),
  listMessages: (chatId) => invoke<ChatMessage[]>('list_messages', { chatId }),
  sendMessage: (chatId, text) => invoke<ChatMessage>('send_message', { chatId, text }),
  setTheme: (theme) => invoke<void>('set_theme', { theme }),
};
