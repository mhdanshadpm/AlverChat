import { DEFAULT_SETTINGS, type AppSettings, type ChatMessage, type ConnectionReport, type ExportResult, type OverlaySnapshot, type Peer, type PermissionStatus, type PlatformBridge } from './index';

/**
 * MOCK MODE. Selected with `?mock=1`, and `?mock=1&scenario=<name>` for a
 * specific state. It exists so every screen, message type, icon state and
 * connection state can be rendered — and screenshotted — without a LAN, and so
 * the shipped UI never has its own fake data mixed into it.
 */
export type MockScenario =
  | 'default'
  | 'empty'
  | 'scanning'
  | 'no-network'
  | 'firewall'
  | 'reconnecting'
  | 'offline-peer'
  | 'incoming-call'
  | 'active-call'
  | 'group-call'
  | 'screen-share'
  | 'dnd'
  | 'dark';

const search = (): URLSearchParams =>
  typeof window === 'undefined' ? new URLSearchParams() : new URLSearchParams(window.location.search);

export function mockScenario(): MockScenario {
  const raw = search().get('scenario');
  const known: MockScenario[] = ['default','empty','scanning','no-network','firewall','reconnecting','offline-peer','incoming-call','active-call','group-call','screen-share','dnd','dark'];
  return known.includes(raw as MockScenario) ? (raw as MockScenario) : 'default';
}

const time = (minutesAgo: number): string => {
  const at = new Date(Date.now() - minutesAgo * 60_000);
  return at.toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });
};

const PEOPLE: Peer[] = [
  { id: 'peer-arjun', name: 'Arjun Kumar', initials: 'AK', color: 'blue', online: true, status: 'Online', preview: 'Invoice updated. Please check.', time: time(4), unread: 3 },
  { id: 'peer-fathima', name: 'Fathima P', initials: 'FP', color: 'rose', online: true, status: 'Online', preview: 'Can you share the stock report?', time: time(11), unread: 2 },
  { id: 'peer-sarath', name: 'Sarath Shailan', initials: 'SS', color: 'green', online: true, status: 'Active now', preview: 'Video ready, please review.', time: time(22) },
  { id: 'peer-samyuktha', name: 'Samyuktha S', initials: 'SK', color: 'peach', online: true, status: 'Active now', preview: 'Looks great!', time: time(35) },
  { id: 'peer-vishnu', name: 'Vishnu K', initials: 'VK', color: 'purple', online: false, status: 'Last seen 5 min ago', preview: 'Noted, thanks.', time: time(300) },
  { id: 'peer-anu', name: 'Anu M', initials: 'AN', color: 'cyan', online: false, status: 'Last seen 32 min ago', preview: 'Adding it to the list.', time: time(32) },
  { id: 'peer-deepa', name: 'Deepa R', initials: 'DP', color: 'yellow', online: true, status: 'In a meeting', preview: 'Report is attached.', time: time(48), muted: true },
];

const GROUPS: Peer[] = [
  { id: 'chat-billing', name: 'Billing Team', initials: 'BT', color: 'violet', online: true, status: '12 members', preview: 'Anu: Purchase entry completed.', time: time(8), group: true, unread: 5, pinned: true },
  { id: 'chat-design', name: 'Design Team', initials: 'DT', color: 'green', online: true, status: '6 members', preview: 'You: New poster added.', time: time(46), group: true },
  { id: 'chat-alverconnect', name: 'Project AlverConnect', initials: 'PA', color: 'peach', online: true, status: '9 members', preview: 'Design review at 4 PM.', time: time(66), group: true, pinned: true },
  { id: 'everyone', name: 'Everyone', initials: 'EV', color: 'blue', online: true, status: 'All members (23)', preview: 'Office network maintenance today.', time: time(120), group: true },
];

const msg = (over: Partial<ChatMessage> & Pick<ChatMessage, 'id' | 'sender' | 'text'>): ChatMessage => ({
  time: time(30), mine: false, read: true, kind: 'text', ...over,
});

const CONVERSATIONS: Record<string, ChatMessage[]> = {
  'peer-arjun': [
    msg({ id: 'm1', sender: 'Arjun Kumar', text: 'Invoice updated. Please check.', time: time(120) }),
    msg({ id: 'm2', sender: 'You', text: 'Yes, I’ll review and confirm in 10 minutes.', time: time(118), mine: true, deliveryStatus: 'delivered' }),
    msg({ id: 'm3', sender: 'Arjun Kumar', text: 'Also, update the stock report.', time: time(90) }),
    msg({ id: 'm4', sender: 'You', text: 'Noted. I’ll update it.', time: time(88), mine: true, deliveryStatus: 'pending' }),
    msg({ id: 'm5', sender: 'Arjun Kumar', text: 'Purchase entry completed for today.', time: time(60) }),
    msg({ id: 'm6', sender: 'Arjun Kumar', text: 'This message was deleted', time: time(58), kind: 'deleted' }),
    msg({ id: 'm7', sender: 'Arjun Kumar', text: 'Stock_Report.xlsx', fileName: 'Stock_Report.xlsx', kind: 'file', time: time(20) }),
  ],
  'chat-billing': [
    msg({ id: 'g1', sender: 'Anu', text: 'Purchase entry completed for today.' }),
    msg({ id: 'g2', sender: 'Sarath', text: 'Original message', time: time(40) }),
    msg({ id: 'g3', sender: 'Fathima', text: 'Looks great!', time: time(30) }),
    msg({ id: 'g4', sender: 'You', text: 'Invoice_ML00037.pdf', fileName: 'Invoice_ML00037.pdf', kind: 'file', mine: true, time: time(12) }),
    msg({ id: 'g5', sender: 'Samyuktha', text: 'Here is the updated design for the invoice.', time: time(9) }),
    msg({ id: 'g6', sender: 'System', text: 'Arjun changed the group icon', kind: 'event', time: time(6) }),
    msg({ id: 'g7', sender: 'System', text: 'Missed voice call at 11:26 AM', kind: 'event', time: time(5) }),
    msg({ id: 'g8', sender: 'System', text: 'Voice call • 12 min 14 sec', kind: 'event', time: time(4) }),
  ],
};

const rich = (chatId: string): ChatMessage[] => CONVERSATIONS[chatId] ?? [
  msg({ id: `${chatId}-1`, sender: 'You', text: 'Send a message to start this conversation.', mine: true, time: time(1) }),
];

let settings: AppSettings = { ...DEFAULT_SETTINGS, displayName: 'Muhammad Anshad', hotkey: 'CmdOrCtrl+Alt+S' };
let peers: Peer[] = [...GROUPS, ...PEOPLE];
let seq = 0;

function scenarioPeers(): Peer[] {
  const scenario = mockScenario();
  if (scenario === 'empty' || scenario === 'scanning' || scenario === 'no-network') return [];
  if (scenario === 'offline-peer') return peers.map(peer => ({ ...peer, online: false, status: 'Last seen just now' }));
  if (scenario === 'firewall') return peers.slice(0, 1);
  return peers;
}

function snapshot(): OverlaySnapshot {
  const list = scenarioPeers();
  return {
    peers: list,
    unreadTotal: list.reduce((total, peer) => total + (peer.unread ?? 0), 0),
    open: false,
    dockSide: settings.screenCorner.startsWith('left') ? 'left' : 'right',
    snapToEdge: settings.snapToEdge,
    bubblesEnabled: settings.bubblesEnabled && !settings.notificationsMuted,
    bubbleAutoHideSeconds: settings.bubbleAutoHideSeconds,
  };
}

const delay = <T,>(value: T): Promise<T> => new Promise(resolve => setTimeout(() => resolve(value), 40));

export const mockBridge: PlatformBridge = {
  ...emptyMockMethods(),
  listPeers: () => delay(scenarioPeers()),
  listMessages: (chatId) => delay(rich(chatId)),
  sendMessage: (chatId, text) => {
    const created: ChatMessage = { id: `out-${++seq}`, sender: 'You', text, time: time(0), mine: true, read: true, kind: 'text', deliveryStatus: 'delivered' };
    const next = [...(CONVERSATIONS[chatId] ?? []), created];
    CONVERSATIONS[chatId] = next;
    peers = peers.map(peer => (peer.id === chatId ? { ...peer, preview: text, time: created.time } : peer));
    return delay(created);
  },
  overlaySnapshot: () => delay(snapshot()),
  markAsRead: (chatId) => { peers = peers.map(peer => (peer.id === chatId ? { ...peer, unread: undefined } : peer)); return delay(undefined); },
  deleteMessageLocal: (messageId) => { for (const list of Object.values(CONVERSATIONS)) { const hit = list.find(item => item.id === messageId); if (hit) hit.kind = 'deleted'; } return delay(true); },
  editMessageLocal: (messageId, text) => { for (const list of Object.values(CONVERSATIONS)) { const hit = list.find(item => item.id === messageId); if (hit) { hit.text = text; hit.edited = true; } } return delay(true); },
  getSettings: () => delay(settings),
  updateSettings: (patch) => { settings = { ...settings, ...patch }; return delay(settings); },
  setLaunchAtLogin: (enabled) => { settings = { ...settings, launchAtLogin: enabled }; return delay(enabled); },
  setHotkey: (combo) => { settings = { ...settings, hotkey: combo }; return delay(combo); },
  isDndActive: () => delay(mockScenario() === 'dnd'),
  setLanPassphrase: (passphrase) => { settings = { ...settings, lanEncryption: passphrase.length > 0 }; return delay(passphrase.length > 0); },
  hasLanPassphrase: () => delay(settings.lanEncryption),
  permissionStatus: (kind): Promise<PermissionStatus> => delay({
    kind,
    label: kind === 'local-network' ? 'Local network access' : kind === 'camera' ? 'Camera' : kind === 'microphone' ? 'Microphone' : kind,
    hint: 'Mock permission status for design review.',
    state: kind === 'microphone' && mockScenario() === 'firewall' ? 'denied' : 'granted',
    deepLink: 'mock://settings',
  }),
  openPermissionSettings: () => delay(undefined),
  openFirewallGuide: () => delay([
    'Open Windows Security → Firewall & network protection.',
    'Click Allow an app through firewall.',
    'Find "AlverChat" in the list and allow it for Private networks (LAN).',
  ]),
  runConnectionTest: (): Promise<ConnectionReport> => {
    const list = scenarioPeers();
    const scenario = mockScenario();
    return delay({
      listening: scenario !== 'no-network',
      port: scenario === 'no-network' ? 0 : 51_234,
      peers: list.length,
      onlinePeers: list.filter(peer => peer.online).length,
      adapters: ['Ethernet (192.168.1.24)', 'Wi-Fi (192.168.1.88)'],
      discoveryPort: 47_986,
      warnings: scenario === 'no-network' ? ['You are not connected to a network.']
        : scenario === 'firewall' ? ['AlverChat is blocked by your firewall.']
        : scenario === 'reconnecting' ? ['Trying to restore your connection to the network.'] : [],
    });
  },
  createBackup: (passphrase) => delay({ path: `mock://Backups/alverchat-backup-${Date.now()}.alverbak`, bytes: passphrase.length * 512, createdMs: Date.now() }),
  listBackups: () => delay([
    { path: 'mock://Backups/alverchat-backup-20261113-094012.alverbak', bytes: 2_411_752, createdMs: Date.now() - 3_600_000 },
    { path: 'mock://Backups/alverchat-backup-20261112-213045.alverbak', bytes: 2_288_104, createdMs: Date.now() - 90_000_000 },
  ]),
  restoreBackup: () => delay(undefined),
  exportData: (): Promise<ExportResult> => delay({ path: 'mock://Exports/alverchat-export.json', messages: Object.values(CONVERSATIONS).reduce((total, list) => total + list.length, 0) }),
  revealPath: (_path: string) => delay(undefined),
  setMutedChat: (chatId, muted) => { peers = peers.map(peer => (peer.id === chatId ? { ...peer, muted } : peer)); settings = { ...settings, mutedChats: muted ? [...new Set([...settings.mutedChats, chatId])] : settings.mutedChats.filter(id => id !== chatId) }; return delay(settings.mutedChats); },
};

/** Windows-only stubs: the mock never opens native overlay windows. */
function emptyMockMethods(): Pick<PlatformBridge,
  'toggleCompact' | 'hideCompact' | 'openFullApp' | 'setOverlayHitRegions' | 'setOverlayPointerCapture'
  | 'startOverlayDrag' | 'moveOverlayBy' | 'endOverlayDrag' | 'resizeDock' | 'openDockMenu' | 'hideMenu'
  | 'quit' | 'resetOverlayPosition' | 'setSnapToEdge' | 'setBubbleSettings' | 'getMyIdentity'> {
  return {
    async toggleCompact() {},
    async hideCompact() {},
    async openFullApp() {},
    async setOverlayHitRegions() {},
    async setOverlayPointerCapture() {},
    async startOverlayDrag() {},
    async moveOverlayBy() {},
    async endOverlayDrag() {},
    async resizeDock() {},
    async openDockMenu() {},
    async hideMenu() {},
    async quit() {},
    async resetOverlayPosition() {},
    async setSnapToEdge() {},
    async setBubbleSettings() {},
    getMyIdentity: () => Promise.resolve({ peerId: 'mock-peer', fingerprint: 'MOCK FINGERPRINT 7Q2P' }),
  };
}
