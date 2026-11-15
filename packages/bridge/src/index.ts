export const APP_NAME = "AlverChat";
/** `presence` and `statusLine` are what the peer itself declared on the wire; `status`
 * is our own liveness copy, so a hidden status still has a sensible line under the name. */
export type Peer = { id:string; name:string; initials:string; color:string; online:boolean; status:string; presence?:string; statusLine?:string; preview:string; time:string; lastMessageMs?:number; unread?:number; group?:boolean; typing?:boolean; muted?:boolean; pinned?:boolean };
export type ChatMessage = { id:string; sender:string; text:string; time:string; timestampMs?:number; mine?:boolean; read?:boolean; deliveryStatus?:'pending'|'delivered'; kind?:'text'|'file'|'event'|'deleted'; fileName?:string; edited?:boolean; forwarded?:boolean; replyTo?:string };
/** One unacknowledged announcement the dock icon may surface. */
export type DockAnnouncement = { id:string; groupId:string; groupName:string; authorName:string; text:string; createdMs:number };
/** A call the shell has seen signalled but not seen end. `startedMs` is 0 while it rings. */
export type LiveCall = { chatId:string; peerName:string; ongoing:boolean; startedMs:number };
export type OverlaySnapshot = { peers:Peer[]; unreadTotal:number; open:boolean; dockSide:'left'|'right'; snapToEdge:boolean; bubblesEnabled:boolean; bubbleAutoHideSeconds:number; showMessagePreview:boolean; opacityPercent:number; idleIcon:'logo'|'photo'; iconMorphEnabled:boolean; iconMorphDelaySeconds:number; notificationsQuiet:boolean; liveCall:LiveCall|null; announcements:DockAnnouncement[] };

/** A signed group roster as the storage layer keeps it (the epoch guards replay). */
export type Group = {
  id:string;
  name:string;
  description?:string;
  /** Emoji plus palette colour, or a data URL when a picture was uploaded. */
  icon?:string;
  epoch:number;
  admin:string;
  members:string[];
  /** `everyone` is the automatic all-peers group: roster and admin are fixed. */
  isEveryone?:boolean;
  isAdmin?:boolean;
};
export type GroupPatch = { name?:string; description?:string; icon?:string };

/** File transfers travel over the same encrypted session as messages. */
export type AttachmentKind = 'image'|'video'|'audio'|'file';
export type TransferState = 'idle'|'queued'|'uploading'|'downloading'|'paused'|'done'|'failed'|'cancelled';
export type Attachment = {
  id:string;
  chatId:string;
  /** The text message this transfer belongs to, when it was sent with a caption. */
  messageId?:string;
  kind:AttachmentKind;
  name:string;
  bytes:number;
  mime:string;
  state:TransferState;
  /** 0-1 while a transfer is running; `null` once complete. */
  progress:number|null;
  transferRateBytesPerSecond:number;
  durationMs?:number;
};

/**
 * Who this device is. `peerId` is what the wire puts in a `from` field; `fingerprint` is
 * the digest a user reads out over another channel to prove a device is the one they mean.
 * They are not the same string and the desktop says so with one struct on both sides.
 */
export type Identity = { peerId:string; fingerprint:string };

/** One emoji with the senders behind it, so reactions render without a second round trip. */
export type ReactionTally = { emoji:string; peers:string[]; mine:boolean };
export type PinnedMessage = { messageId:string; chatId:string; sender:string; text:string; pinnedMs:number; pinnedBy:string };
export type Announcement = { id:string; groupId:string; author:string; authorName:string; text:string; createdMs:number; acknowledgedBy:string[] };
export type CallStatus = 'missed'|'declined'|'ended'|'ringing'|'ongoing'|'failed';
export type CallEvent = { id:string; chatId:string; kind:'1to1'|'group'; status:CallStatus; caller:string; callerName:string; startedMs:number; endedMs:number|null; participants:string[] };
/** WebRTC signalling relayed over the encrypted peer session; SDP and ICE stay opaque. */
export type CallSignal = {
  callId:string;
  kind:'offer'|'answer'|'ice'|'end'|'reject'|'busy';
  /** `audio`, `video` or `screen`, present on an offer. */
  media?:string;
  sdp?:string;
  candidate?:string;
  sdpMid?:string|null;
  sdpIndex?:number|null;
  reason?:string|null;
  accepted?:boolean;
};
export type IncomingCallSignal = { chatId:string; caller:string; signal:CallSignal };
export type SearchHit = { chatId:string; chatName:string; messageId:string; sender:string; text:string; timestampMs:number };
export type UnreadState = { chatId:string; unread:number; firstUnreadId:string|null };

/** Mirrors `platform::AppSettings` in the desktop shell. */
export type AppSettings = {
  theme:'light'|'dark'|'system';
  language:string;
  launchAtLogin:boolean;
  displayName:string;
  avatarColor:string;
  presence:string;
  customStatus:string;
  bubblesEnabled:boolean;
  idleIcon:'logo'|'photo';
  iconMorphEnabled:boolean;
  iconMorphDelaySeconds:number;
  bubbleAutoHideSeconds:number;
  screenCorner:string;
  opacityPercent:number;
  snapToEdge:boolean;
  hotkey:string;
  notificationSound:boolean;
  notificationsMuted:boolean;
  dndEnabled:boolean;
  dndDays:string[];
  dndFrom:string;
  dndTo:string;
  mutedChats:string[];
  /** Chats pinned to the top of the list, in the order they were pinned. */
  pinnedChats:string[];
  showMessagePreview:boolean;
  editWindowMinutes:number;
  autoDownloadLimitMb:number;
  maxFileSizeMb:number;
  downloadsDir:string;
  microphone:string;
  speaker:string;
  camera:string;
  noiseSuppression:boolean;
  echoCancellation:boolean;
  ringtone:string;
  lanEncryption:boolean;
  whoSeesStatus:string;
  visibleOnNetwork:boolean;
  networkAdapter:string;
  announcementsFromAdminsOnly:boolean;
};
export type PermissionStatus = { kind:string; label:string; hint:string; state:'granted'|'denied'|'unknown'; deepLink:string };
export type ConnectionReport = { listening:boolean; port:number; peers:number; onlinePeers:number; adapters:string[]; discoveryPort:number; discoveryActive:boolean; warnings:string[] };
export type BackupResult = { path:string; bytes:number; createdMs:number };
export type ExportResult = { path:string; messages:number };

export interface PlatformBridge {
  listPeers(): Promise<Peer[]>;
  listMessages(chatId:string): Promise<ChatMessage[]>;
  /** `replyToId` quotes an earlier message; the desktop stores the quote in `messages.reply_to` (migration v5). */
  sendMessage(chatId:string,text:string,replyToId?:string): Promise<ChatMessage>;
  overlaySnapshot(): Promise<OverlaySnapshot>;
  toggleCompact(): Promise<void>;
  hideCompact(): Promise<void>;
  openFullApp(chatId?:string): Promise<void>;
  setOverlayHitRegions(hasHit:boolean): Promise<void>;
  setOverlayPointerCapture(active:boolean,hasHit:boolean): Promise<void>;
  startOverlayDrag(): Promise<void>;
  moveOverlayBy(deltaX:number,deltaY:number): Promise<void>;
  endOverlayDrag(hasHit:boolean,reduceMotion:boolean): Promise<void>;
  resizeDock(hasBubbles:boolean): Promise<void>;
  openDockMenu(): Promise<void>;
  hideMenu(): Promise<void>;
  quit(): Promise<void>;
  resetOverlayPosition(): Promise<void>;
  setSnapToEdge(enabled:boolean): Promise<void>;
  setBubbleSettings(enabled:boolean,autoHideSeconds:number): Promise<void>;
  markAsRead(chatId:string): Promise<void>;
  deleteMessageLocal(messageId:string): Promise<boolean>;
  /** Unsends for every participant; only the original sender may do it, inside the edit window. */
  deleteForEveryone(messageId:string): Promise<boolean>;
  editMessageLocal(messageId:string,text:string): Promise<boolean>;
  getMyIdentity(): Promise<Identity>;
  getSettings(): Promise<AppSettings>;
  updateSettings(patch:Partial<AppSettings>): Promise<AppSettings>;
  setLaunchAtLogin(enabled:boolean): Promise<boolean>;
  setHotkey(combo:string): Promise<string>;
  isDndActive(): Promise<boolean>;
  setLanPassphrase(passphrase:string): Promise<boolean>;
  hasLanPassphrase(): Promise<boolean>;
  permissionStatus(kind:string): Promise<PermissionStatus>;
  openPermissionSettings(kind:string): Promise<void>;
  openFirewallGuide(): Promise<string[]>;
  runConnectionTest(): Promise<ConnectionReport>;
  createBackup(passphrase:string): Promise<BackupResult>;
  listBackups(): Promise<BackupResult[]>;
  restoreBackup(path:string,passphrase:string): Promise<void>;
  exportData(): Promise<ExportResult>;
  revealPath(path:string): Promise<void>;
  /** Message-level richness. Every one of these is local-only until the peer answers. */
  pinMessage(chatId:string, messageId:string, pinned:boolean): Promise<PinnedMessage[]>;
  pinnedMessages(chatId:string): Promise<PinnedMessage[]>;
  starMessage(messageId:string, starred:boolean): Promise<boolean>;
  starredMessages(chatId:string): Promise<ChatMessage[]>;
  reactionsFor(messageId:string): Promise<ReactionTally[]>;
  /** Toggles the caller's own reaction; sending the same emoji again removes it. */
  react(messageId:string, emoji:string): Promise<ReactionTally[]>;
  /** Copies messages into other conversations as new outgoing messages; returns how many landed. */
  forwardMessages(messageIds:string[], chatIds:string[]): Promise<number>;
  searchMessages(query:string, chatId?:string): Promise<SearchHit[]>;
  setTyping(chatId:string, typing:boolean): Promise<void>;
  markUnread(chatId:string): Promise<UnreadState[]>;
  clearChat(chatId:string): Promise<void>;
  deleteChat(chatId:string): Promise<void>;
  attachmentsFor(chatId:string): Promise<Attachment[]>;
  /** `dataUrl` is base64 only for small files; larger transfers stream from disk. */
  sendFile(chatId:string, fileName:string, dataUrl:string, durationMs?:number): Promise<Attachment>;
  controlTransfer(attachmentId:string, action:'pause'|'resume'|'retry'|'cancel'): Promise<Attachment|null>;
  openAttachment(attachmentId:string): Promise<string|null>;
  /** A completed attachment as a data URL, so voice notes can play inside the app. */
  attachmentData(attachmentId:string): Promise<string|null>;
  listAnnouncements(groupId:string): Promise<Announcement[]>;
  sendAnnouncement(groupId:string, text:string): Promise<Announcement>;
  acknowledgeAnnouncement(announcementId:string): Promise<Announcement|null>;
  listCalls(chatId?:string): Promise<CallEvent[]>;
  /** Relays one WebRTC signalling frame to a peer over the encrypted session. */
  sendCallSignal(chatId:string, signal:CallSignal): Promise<void>;
  setMutedChat(chatId:string,muted:boolean): Promise<string[]>;
  setPinnedChat(chatId:string,pinned:boolean): Promise<string[]>;
  listGroups(): Promise<Group[]>;
  createGroup(name:string, memberIds:string[], icon?:string): Promise<Group>;
  updateGroup(id:string, patch:GroupPatch): Promise<Group>;
  addGroupMembers(id:string, memberIds:string[]): Promise<Group>;
  removeGroupMember(id:string, peerId:string): Promise<Group>;
  leaveGroup(id:string): Promise<void>;
  /** Stores a 256 px, EXIF-stripped JPEG for my peer card; `null` removes it. */
  setAvatar(avatarDataUrl:string|null): Promise<string|null>;
  getAvatar(): Promise<string|null>;
  /** The photo a peer publishes, pulled over the LAN; `null` when they publish none. */
  getPeerAvatar(peerId:string): Promise<string|null>;
}

export const DEFAULT_SETTINGS: AppSettings = {
  theme:'system', language:'en', launchAtLogin:false, displayName:'', avatarColor:'violet',
  presence:'Available', customStatus:'', bubblesEnabled:true, idleIcon:'logo',
  iconMorphEnabled:true, iconMorphDelaySeconds:5, bubbleAutoHideSeconds:7,
  screenCorner:'right-center', opacityPercent:90, snapToEdge:true,
  hotkey:'Ctrl+Alt+S', notificationSound:true, notificationsMuted:false, dndEnabled:true,
  dndDays:['Mon','Tue','Wed','Thu','Fri'], dndFrom:'22:00', dndTo:'07:00', mutedChats:[], pinnedChats:[],
  showMessagePreview:true, editWindowMinutes:15, autoDownloadLimitMb:50, maxFileSizeMb:500,
  downloadsDir:'', microphone:'default', speaker:'default', camera:'default',
  noiseSuppression:true, echoCancellation:true, ringtone:'alverchat', lanEncryption:false,
  whoSeesStatus:'everyone', visibleOnNetwork:true, networkAdapter:'auto',
  announcementsFromAdminsOnly:false,
};

const unavailable = (feature:string):Promise<never> =>
  Promise.reject(new Error(`${feature} is only available in the AlverChat desktop app.`));

export const emptyBrowserBridge:PlatformBridge = {
 async listPeers(){return []},
 async listMessages(){return []},
 async sendMessage(){return unavailable('Messaging')},
 async overlaySnapshot(){return {peers:[],unreadTotal:0,open:false,dockSide:'right',snapToEdge:true,bubblesEnabled:true,bubbleAutoHideSeconds:7,showMessagePreview:true,opacityPercent:90,idleIcon:'logo',iconMorphEnabled:true,iconMorphDelaySeconds:5,notificationsQuiet:false,liveCall:null,announcements:[]}},
 async toggleCompact(){/* browser preview */},
 async hideCompact(){/* browser preview */},
 async openFullApp(){/* browser preview */},
 async setOverlayHitRegions(){/* browser preview */},
 async setOverlayPointerCapture(){/* browser preview */},
 async startOverlayDrag(){/* browser preview */},
 async moveOverlayBy(){/* browser preview */},
 async endOverlayDrag(){/* browser preview */},
 async resizeDock(){/* browser preview */},
 async openDockMenu(){/* browser preview */},
 async hideMenu(){/* browser preview */},
 async quit(){/* browser preview */},
 async resetOverlayPosition(){/* browser preview */},
 async setSnapToEdge(){/* browser preview */},
 async setBubbleSettings(){/* browser preview */},
 async markAsRead(){/* browser preview */},
 async deleteMessageLocal(){return true},
 async deleteForEveryone(){return true},
 async editMessageLocal(){return true},
 async getMyIdentity(){return { peerId: 'browser-preview', fingerprint: 'preview-fingerprint' }},
 async getSettings(){return {...DEFAULT_SETTINGS}},
 async updateSettings(patch){return {...DEFAULT_SETTINGS, ...patch}},
 async setLaunchAtLogin(enabled){return enabled},
 async setHotkey(combo){return combo},
 async isDndActive(){return false},
 async setLanPassphrase(passphrase){return passphrase.length>0},
 async hasLanPassphrase(){return false},
 async permissionStatus(kind){return {kind, label:kind, hint:'', state:'unknown', deepLink:''}},
 async openPermissionSettings(){return unavailable('System settings')},
 async openFirewallGuide(){return []},
 async runConnectionTest(){return {listening:false,port:0,peers:0,onlinePeers:0,adapters:[],discoveryPort:47986,discoveryActive:false,warnings:['Running outside the desktop app.']}},
 async createBackup(){return unavailable('Backups')},
 async listBackups(){return []},
 async restoreBackup(){return unavailable('Restore')},
 async exportData(){return unavailable('Export')},
 async revealPath(){/* browser preview */},
 async setMutedChat(_chatId,muted){return muted?['muted']:[]},
 async setPinnedChat(_chatId,pinned){return pinned?['pinned']:[]},
 async listGroups(){return []},
 async createGroup(){return unavailable('Groups')},
 async updateGroup(){return unavailable('Groups')},
 async addGroupMembers(){return unavailable('Groups')},
 async removeGroupMember(){return unavailable('Groups')},
 async leaveGroup(){return unavailable('Groups')},
 async setAvatar(){return null},
 async getAvatar(){return null},
 async getPeerAvatar(){return null},
 async pinMessage(){return []},
 async pinnedMessages(){return []},
 async starMessage(){return true},
 async starredMessages(){return []},
 async reactionsFor(){return []},
 async react(){return []},
 async forwardMessages(){return 0},
 async searchMessages(){return []},
 async setTyping(){/* browser preview */},
 async markUnread(){return []},
 async clearChat(){/* browser preview */},
 async deleteChat(){/* browser preview */},
 async attachmentsFor(){return []},
 async sendFile(){return unavailable('File transfers')},
 async controlTransfer(){return null},
 async openAttachment(){return null},
 async attachmentData(){return null},
 async listAnnouncements(){return []},
 async sendAnnouncement(){return unavailable('Announcements')},
 async acknowledgeAnnouncement(){return null},
 async listCalls(){return []},
 async sendCallSignal(){return unavailable('Calls')},
};

export async function bridgeForRuntime():Promise<PlatformBridge>{
  if (typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window) {
    const { tauriBridge } = await import('./tauri');
    return tauriBridge;
  }
  if (typeof window !== 'undefined' && new URLSearchParams(window.location.search).get('mock') === '1') {
    const { mockBridge } = await import('./mock');
    return mockBridge;
  }
  return emptyBrowserBridge;
}
