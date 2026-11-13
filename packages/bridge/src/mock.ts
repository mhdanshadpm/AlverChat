import { DEFAULT_SETTINGS, type Announcement, type AppSettings, type Attachment, type CallEvent, type ChatMessage, type ConnectionReport, type ExportResult, type Group, type GroupPatch, type OverlaySnapshot, type PinnedMessage, type ReactionTally, type SearchHit, type UnreadState, type Peer, type PermissionStatus, type PlatformBridge } from './index';

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

const ago = (minutesAgo: number): number => Date.now() - minutesAgo * 60_000;

const clock = (ms: number): string => new Date(ms).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });

const time = (minutesAgo: number): string => clock(ago(minutesAgo));

/** Stamp pair, so a fixture sorts by time as well as displaying one. */
const at = (minutesAgo: number): { time: string; timestampMs: number } => ({
  time: time(minutesAgo),
  timestampMs: ago(minutesAgo),
});

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
  ...at(30), mine: false, read: true, kind: 'text', ...over,
});

const CONVERSATIONS: Record<string, ChatMessage[]> = {
  'peer-arjun': [
    msg({ id: 'm1', sender: 'Arjun Kumar', text: 'Invoice updated. Please check.', ...at(120) }),
    msg({ id: 'm2', sender: 'You', text: 'Yes, I’ll review and confirm in 10 minutes.', ...at(118), mine: true, deliveryStatus: 'delivered' }),
    msg({ id: 'm3', sender: 'Arjun Kumar', text: 'Also, update the stock report.', ...at(90) }),
    msg({ id: 'm4', sender: 'You', text: 'Noted. I’ll update it.', ...at(88), mine: true, deliveryStatus: 'pending' }),
    msg({ id: 'm5', sender: 'Arjun Kumar', text: 'Purchase entry completed for today.', ...at(60) }),
    msg({ id: 'm6', sender: 'Arjun Kumar', text: 'This message was deleted', ...at(58), kind: 'deleted' }),
    msg({ id: 'm7', sender: 'Arjun Kumar', text: 'Stock_Report.xlsx', fileName: 'Stock_Report.xlsx', kind: 'file', ...at(20) }),
  ],
  'chat-billing': [
    msg({ id: 'g1', sender: 'Anu', text: 'Purchase entry completed for today.' }),
    msg({ id: 'g2', sender: 'Sarath', text: 'Original message', ...at(40) }),
    msg({ id: 'g3', sender: 'Fathima', text: 'Looks great!', ...at(30) }),
    msg({ id: 'g4', sender: 'You', text: 'Invoice_ML00037.pdf', fileName: 'Invoice_ML00037.pdf', kind: 'file', mine: true, ...at(12) }),
    msg({ id: 'g5', sender: 'Samyuktha', text: 'Here is the updated design for the invoice.', ...at(9) }),
    msg({ id: 'g6', sender: 'System', text: 'Arjun changed the group icon', kind: 'event', ...at(6) }),
    msg({ id: 'g7', sender: 'System', text: 'Missed voice call at 11:26 AM', kind: 'event', ...at(5) }),
    msg({ id: 'g8', sender: 'System', text: 'Voice call • 12 min 14 sec', kind: 'event', ...at(4) }),
  ],
};

const rich = (chatId: string): ChatMessage[] => CONVERSATIONS[chatId] ?? [
  msg({ id: `${chatId}-1`, sender: 'You', text: 'Send a message to start this conversation.', mine: true, ...at(1) }),
];

let settings: AppSettings = { ...DEFAULT_SETTINGS, displayName: 'Muhammad Anshad', hotkey: 'CmdOrCtrl+Alt+S', pinnedChats: ['chat-alverconnect', 'chat-billing'] };
let peers: Peer[] = [...GROUPS, ...PEOPLE];
let seq = 0;

let groups: Group[] = [
  { id: 'everyone', name: 'Everyone', epoch: 1, admin: 'peer-arjun', members: ['peer-arjun', 'peer-fathima', 'peer-sarath', 'peer-samyuktha', 'peer-vishnu', 'peer-anu', 'peer-deepa'], isEveryone: true, isAdmin: true, description: 'Automatic group for every peer found on this network.' },
  { id: 'chat-billing', name: 'Billing Team', epoch: 3, admin: 'me', members: ['me', 'peer-anu', 'peer-arjun', 'peer-deepa', 'peer-fathima'], isAdmin: true, icon: '\u{1F4B0}', description: 'Invoices, purchase entries and the monthly stock report.' },
  { id: 'chat-design', name: 'Design Team', epoch: 1, admin: 'peer-sarath', members: ['me', 'peer-sarath', 'peer-samyuktha'], icon: '\u{1F3A8}' },
  { id: 'chat-alverconnect', name: 'Project AlverConnect', epoch: 2, admin: 'me', members: ['me', 'peer-arjun', 'peer-fathima', 'peer-sarath', 'peer-samyuktha'], isAdmin: true, icon: '\u{1F680}', description: 'Launch room for the AlverConnect rollout.' },
];
let avatar: string | null = null;

const groupMembers = (group: Group): string[] => group.members;
const initialsFrom = (name: string): string => name.trim().split(/\s+/).slice(0, 2).map(word => word[0]?.toUpperCase() ?? '').join('') || 'G';

function upsertGroupPeer(group: Group): void {
  const existing = peers.find(peer => peer.id === group.id);
  const next: Peer = {
    id: group.id,
    name: group.name,
    initials: initialsFrom(group.name),
    color: existing?.color ?? 'violet',
    online: true,
    status: `${groupMembers(group).length} members`,
    preview: existing?.preview ?? 'Group created',
    time: existing?.time ?? time(0),
    unread: existing?.unread,
    group: true,
    pinned: existing?.pinned,
  };
  peers = existing ? peers.map(peer => (peer.id === group.id ? next : peer)) : [next, ...peers];
}

const saveGroup = (group: Group): Group => {
  groups = groups.map(item => (item.id === group.id ? group : item));
  upsertGroupPeer(group);
  return group;
};

/**
 * Rich-content state. Kept separate from `CONVERSATIONS` so the message list
 * stays a plain fixture and the extra surfaces (reactions, pins, transfers,
 * announcements, call history) can be mutated by the UI like real storage.
 */
const REACTIONS: Record<string, ReactionTally[]> = {
  g3: [{ emoji: '👍', peers: ['peer-anu', 'peer-sarath'], mine: false }, { emoji: '❤️', peers: ['peer-samyuktha'], mine: false }],
  m5: [{ emoji: '🎉', peers: ['peer-arjun'], mine: true }],
  g1: [{ emoji: '👀', peers: ['peer-deepa'], mine: false }],
};
const PINNED: Record<string, PinnedMessage[]> = {
  'chat-billing': [{ messageId: 'g1', chatId: 'chat-billing', sender: 'Anu', text: 'Purchase entry completed for today.', pinnedMs: Date.now() - 3_600_000, pinnedBy: 'me' }],
  'peer-arjun': [{ messageId: 'm5', chatId: 'peer-arjun', sender: 'Arjun Kumar', text: 'Purchase entry completed for today.', pinnedMs: Date.now() - 7_200_000, pinnedBy: 'peer-arjun' }],
};
const STARRED = new Set<string>(['g5', 'm2']);

const ATTACHMENTS: Record<string, Attachment[]> = {
  'peer-arjun': [
    { id: 'a1', chatId: 'peer-arjun', messageId: 'm7', kind: 'file', name: 'Stock_Report.xlsx', bytes: 4_812_034, mime: 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet', state: 'done', progress: null, transferRateBytesPerSecond: 0, autoDownloaded: true },
    { id: 'a2', chatId: 'peer-arjun', kind: 'file', name: 'GST_Reconciliation_Nov.xlsx', bytes: 18_400_000, mime: 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet', state: 'downloading', progress: 0.42, transferRateBytesPerSecond: 21_500_000 },
  ],
  'peer-sarath': [
    { id: 'a3', chatId: 'peer-sarath', kind: 'video', name: 'launch-film-v3.mp4', bytes: 246_000_000, mime: 'video/mp4', state: 'paused', progress: 0.68, transferRateBytesPerSecond: 0 },
    { id: 'a4', chatId: 'peer-sarath', kind: 'image', name: 'poster-option-a.png', bytes: 2_100_000, mime: 'image/png', state: 'failed', progress: 0.11, transferRateBytesPerSecond: 0 },
    { id: 'a5', chatId: 'peer-sarath', kind: 'audio', name: 'voice-note-02.m4a', bytes: 640_000, mime: 'audio/mp4', state: 'done', progress: null, transferRateBytesPerSecond: 0, durationMs: 42_000 },
  ],
  'chat-design': [
    { id: 'a6', chatId: 'chat-design', kind: 'file', name: 'AlverConnect_Guidelines.pdf', bytes: 9_400_000, mime: 'application/pdf', state: 'uploading', progress: 0.27, transferRateBytesPerSecond: 12_000_000 },
  ],
};

const ANNOUNCEMENTS: Announcement[] = [
  { id: 'an1', groupId: 'everyone', authorName: 'Arjun Kumar', author: 'peer-arjun', text: 'Office network maintenance today from 6 PM to 7 PM. Expect short disconnects.', createdMs: Date.now() - 5_400_000, acknowledgedBy: ['peer-fathima', 'peer-anu'] },
  { id: 'an2', groupId: 'chat-alverconnect', authorName: 'You', author: 'me', text: 'Design review at 4 PM. Please upload final exports before then.', createdMs: Date.now() - 1_800_000, acknowledgedBy: [] },
];

const CALLS: CallEvent[] = [
  { id: 'c1', chatId: 'peer-arjun', kind: '1to1', status: 'missed', caller: 'peer-arjun', callerName: 'Arjun Kumar', startedMs: Date.now() - 7_200_000, endedMs: Date.now() - 7_200_000, participants: ['peer-arjun'] },
  { id: 'c2', chatId: 'peer-arjun', kind: '1to1', status: 'ended', caller: 'me', callerName: 'You', startedMs: Date.now() - 86_400_000, endedMs: Date.now() - 86_400_000 + 734_000, participants: ['me', 'peer-arjun'] },
  { id: 'c3', chatId: 'chat-alverconnect', kind: 'group', status: 'ended', caller: 'me', callerName: 'You', startedMs: Date.now() - 3_600_000, endedMs: Date.now() - 3_600_000 + 1_940_000, participants: ['me', 'peer-arjun', 'peer-fathima', 'peer-sarath'] },
  { id: 'c4', chatId: 'peer-fathima', kind: '1to1', status: 'declined', caller: 'peer-fathima', callerName: 'Fathima P', startedMs: Date.now() - 1_200_000, endedMs: null, participants: ['peer-fathima'] },
];

const typingChats = new Set<string>(['chat-billing']);
const UNREAD = new Map<string, number>();

const chatName = (chatId: string): string =>
  peers.find(peer => peer.id === chatId)?.name ?? chatId;

const findConversation = (messageId: string): ChatMessage[] | undefined => {
  for (const list of Object.values(CONVERSATIONS)) if (list.some(item => item.id === messageId)) return list;
  return undefined;
};

/** The tail of a thread the reader has not seen, mirroring the desktop read receipts. */
const unreadTail = (chatId: string, thread: ChatMessage[]): ChatMessage[] => {
  const wanted = UNREAD.get(chatId) ?? peers.find(peer => peer.id === chatId)?.unread ?? 0;
  if (wanted <= 0) return [];
  return thread.filter(message => !message.mine && message.kind !== 'deleted').slice(-wanted);
};

/** Mock unread counters live here so `markAsRead` can still clear the fixture values. */
const applyUnread = (): Peer[] => {
  peers = peers.map(peer => (UNREAD.has(peer.id) ? { ...peer, unread: UNREAD.get(peer.id) || undefined } : peer));
  return peers;
};

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
  listMessages: (chatId) => {
    const thread = rich(chatId);
    const unseen = new Set(unreadTail(chatId, thread).map(message => message.id));
    return delay(thread.map(message => (unseen.has(message.id) ? { ...message, read: false } : message)));
  },
  sendMessage: (chatId, text, replyToId) => {
    const created: ChatMessage = { id: `out-${++seq}`, sender: 'You', text, ...at(0), mine: true, read: true, kind: 'text', deliveryStatus: 'delivered', replyTo: replyToId };
    const next = [...(CONVERSATIONS[chatId] ?? []), created];
    CONVERSATIONS[chatId] = next;
    peers = peers.map(peer => (peer.id === chatId ? { ...peer, preview: text, time: created.time } : peer));
    return delay(created);
  },
  overlaySnapshot: () => delay(snapshot()),
  markAsRead: (chatId) => { UNREAD.delete(chatId); peers = peers.map(peer => (peer.id === chatId ? { ...peer, unread: undefined } : peer)); return delay(undefined); },
  deleteMessageLocal: (messageId) => { for (const list of Object.values(CONVERSATIONS)) { const hit = list.find(item => item.id === messageId); if (hit) hit.kind = 'deleted'; } return delay(true); },
  deleteForEveryone: (messageId) => {
    for (const list of Object.values(CONVERSATIONS)) {
      const hit = list.find(item => item.id === messageId);
      if (!hit) continue;
      hit.kind = 'deleted';
      hit.text = '';
      hit.fileName = undefined;
      hit.edited = false;
    }
    for (const chatId of Object.keys(PINNED)) {
      PINNED[chatId] = PINNED[chatId].filter(entry => entry.messageId !== messageId);
    }
    STARRED.delete(messageId);
    delete REACTIONS[messageId];
    return delay(true);
  },
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
  revealPath: () => delay(undefined),
  pinMessage: (chatId, messageId, pinned) => {
    const list = PINNED[chatId] ?? (PINNED[chatId] = []);
    const hit = findConversation(messageId)?.find(item => item.id === messageId);
    if (pinned && hit && !list.some(item => item.messageId === messageId)) list.push({ messageId, chatId, sender: hit.mine ? 'You' : hit.sender, text: hit.text, pinnedMs: Date.now(), pinnedBy: 'me' });
    if (!pinned) PINNED[chatId] = list.filter(item => item.messageId !== messageId);
    return delay(PINNED[chatId]);
  },
  pinnedMessages: (chatId) => delay(PINNED[chatId] ?? []),
  starMessage: (messageId, starred) => { if (starred) STARRED.add(messageId); else STARRED.delete(messageId); return delay(starred); },
  starredMessages: (chatId) => delay((CONVERSATIONS[chatId] ?? []).filter(message => STARRED.has(message.id))),
  reactionsFor: (messageId) => delay(REACTIONS[messageId] ?? []),
  react: (messageId, emoji) => {
    const tallies = REACTIONS[messageId] ?? (REACTIONS[messageId] = []);
    const existing = tallies.find(tally => tally.emoji === emoji);
    if (!existing) tallies.push({ emoji, peers: ['me'], mine: true });
    else if (existing.mine) {
      existing.peers = existing.peers.filter(peer => peer !== 'me');
      existing.mine = false;
      if (!existing.peers.length) tallies.splice(tallies.indexOf(existing), 1);
    } else { existing.peers = [...existing.peers, 'me']; existing.mine = true; }
    REACTIONS[messageId] = tallies.filter(tally => tally.peers.length > 0);
    return delay(REACTIONS[messageId]);
  },
  forwardMessages: (messageIds, chatIds) => {
    let copied = 0;
    for (const chatId of chatIds) {
      const list = CONVERSATIONS[chatId] ?? (CONVERSATIONS[chatId] = []);
      for (const messageId of messageIds) {
        const source = Object.values(CONVERSATIONS).flatMap(items => items).find(item => item.id === messageId);
        if (!source) continue;
        list.push(msg({ id: `fwd-${++seq}`, sender: 'You', text: source.text, mine: true, ...at(0), read: true, deliveryStatus: 'delivered', fileName: source.fileName, kind: source.kind === 'file' ? 'file' : 'text' }));
        copied += 1;
      }
      peers = peers.map(peer => (peer.id === chatId ? { ...peer, time: time(0) } : peer));
    }
    return delay(copied);
  },
  searchMessages: (query, chatId) => {
    const needle = query.trim().toLowerCase();
    if (!needle) return delay([]);
    const scope = chatId ? [chatId] : peers.map(peer => peer.id);
    const hits: SearchHit[] = [];
    for (const id of scope) {
      for (const message of CONVERSATIONS[id] ?? []) {
        if (message.text.toLowerCase().includes(needle) || (message.fileName ?? '').toLowerCase().includes(needle))
          hits.push({ chatId: id, chatName: chatName(id), messageId: message.id, sender: message.mine ? 'You' : message.sender, text: message.text, timestampMs: Date.now() });
      }
    }
    return delay(hits.slice(0, 80));
  },
  setTyping: (chatId, typing) => {
    if (typing) typingChats.add(chatId); else typingChats.delete(chatId);
    peers = peers.map(peer => (peer.id === chatId ? { ...peer, typing } : peer));
    return delay(undefined);
  },
  markUnread: (chatId): Promise<UnreadState[]> => {
    UNREAD.set(chatId, (UNREAD.get(chatId) ?? 0) + 1);
    const first = unreadTail(chatId, rich(chatId))[0];
    return delay(applyUnread().map(peer => ({ chatId: peer.id, unread: UNREAD.get(peer.id) ?? peer.unread ?? 0, firstUnreadId: peer.id === chatId ? first?.id ?? null : null })));
  },
  clearChat: (chatId) => { CONVERSATIONS[chatId] = []; delete PINNED[chatId]; return delay(undefined); },
  deleteChat: (chatId) => {
    CONVERSATIONS[chatId] = [];
    delete PINNED[chatId];
    ATTACHMENTS[chatId] = [];
    peers = peers.filter(peer => peer.id !== chatId);
    return delay(undefined);
  },
  attachmentsFor: (chatId) => delay(ATTACHMENTS[chatId] ?? []),
  sendFile: (chatId, fileName, dataUrl) => {
    const kind: Attachment['kind'] = /\.png|\.jpe?g|\.gif|\.webp$/i.test(fileName) ? 'image'
      : /\.mp4|\.mov|\.webm$/i.test(fileName) ? 'video'
      : /\.m4a|\.mp3|\.wav|\.ogg$/i.test(fileName) ? 'audio' : 'file';
    const bytes = Math.max(1, Math.round((dataUrl.length - dataUrl.indexOf(',') - 1) * 0.75)) || 256_000;
    const created: Attachment = { id: `a-${++seq}`, chatId, kind, name: fileName, bytes, mime: 'application/octet-stream', state: 'uploading', progress: 0.05, transferRateBytesPerSecond: 24_000_000 };
    (ATTACHMENTS[chatId] ?? (ATTACHMENTS[chatId] = [])).push(created);
    (CONVERSATIONS[chatId] ?? (CONVERSATIONS[chatId] = [])).push(msg({ id: `file-${seq}`, sender: 'You', text: fileName, mine: true, kind: 'file', fileName, ...at(0), read: true, deliveryStatus: 'delivered' }));
    peers = peers.map(peer => (peer.id === chatId ? { ...peer, preview: fileName, time: time(0) } : peer));
    return delay(created);
  },
  controlTransfer: (attachmentId, action) => {
    for (const list of Object.values(ATTACHMENTS)) {
      const hit = list.find(item => item.id === attachmentId);
      if (!hit) continue;
      if (action === 'cancel') { const index = list.indexOf(hit); list.splice(index, 1); return delay(null); }
      hit.state = action === 'pause' ? 'paused' : action === 'retry' ? 'uploading' : hit.kind === 'audio' || hit.kind === 'image' ? 'downloading' : 'uploading';
      if (action === 'retry') hit.progress = 0.02;
      return delay(hit);
    }
    return delay(null);
  },
  openAttachment: () => delay(null),
  listAnnouncements: (groupId) => delay(ANNOUNCEMENTS.filter(item => item.groupId === groupId)),
  sendAnnouncement: (groupId, text) => {
    const created: Announcement = { id: `an-${++seq}`, groupId, author: 'me', authorName: 'You', text, createdMs: Date.now(), acknowledgedBy: [] };
    ANNOUNCEMENTS.unshift(created);
    return delay(created);
  },
  acknowledgeAnnouncement: (announcementId) => {
    const hit = ANNOUNCEMENTS.find(item => item.id === announcementId);
    if (!hit) return delay(null);
    if (!hit.acknowledgedBy.includes('me')) hit.acknowledgedBy = [...hit.acknowledgedBy, 'me'];
    return delay(hit);
  },
  listCalls: (chatId) => delay(chatId ? CALLS.filter(call => call.chatId === chatId) : [...CALLS]),
  setMutedChat: (chatId, muted) => { peers = peers.map(peer => (peer.id === chatId ? { ...peer, muted } : peer)); settings = { ...settings, mutedChats: muted ? [...new Set([...settings.mutedChats, chatId])] : settings.mutedChats.filter(id => id !== chatId) }; return delay(settings.mutedChats); },
  setPinnedChat: (chatId, pinned) => { peers = peers.map(peer => (peer.id === chatId ? { ...peer, pinned } : peer)); settings = { ...settings, pinnedChats: pinned ? [chatId, ...settings.pinnedChats.filter(id => id !== chatId)] : settings.pinnedChats.filter(id => id !== chatId) }; return delay(settings.pinnedChats); },
  listGroups: () => delay(groups),
  createGroup: (name, memberIds, icon) => {
    const group: Group = { id: `chat-${++seq}`, name, epoch: 1, admin: 'me', members: ['me', ...memberIds], isAdmin: true, icon };
    groups = [...groups, group];
    upsertGroupPeer(group);
    return delay(group);
  },
  updateGroup: (id, patch: GroupPatch) => {
    const group = groups.find(item => item.id === id);
    if (!group) return Promise.reject(new Error(`No group named ${id} in this mock.`));
    return delay(saveGroup({ ...group, ...patch }));
  },
  addGroupMembers: (id, memberIds) => {
    const group = groups.find(item => item.id === id);
    if (!group) return Promise.reject(new Error(`No group named ${id} in this mock.`));
    return delay(saveGroup({ ...group, members: [...new Set([...group.members, ...memberIds])], epoch: group.epoch + 1 }));
  },
  removeGroupMember: (id, peerId) => {
    const group = groups.find(item => item.id === id);
    if (!group) return Promise.reject(new Error(`No group named ${id} in this mock.`));
    if (group.isEveryone) return Promise.reject(new Error('The Everyone group follows the peers visible on the network.'));
    return delay(saveGroup({ ...group, members: group.members.filter(member => member !== peerId), epoch: group.epoch + 1 }));
  },
  leaveGroup: (id) => {
    if (id === 'everyone') return Promise.reject(new Error('You cannot leave the Everyone group.'));
    groups = groups.filter(group => group.id !== id);
    peers = peers.filter(peer => peer.id !== id);
    return delay(undefined);
  },
  setAvatar: (dataUrl) => { avatar = dataUrl; return delay(avatar); },
  getAvatar: () => delay(avatar),
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
