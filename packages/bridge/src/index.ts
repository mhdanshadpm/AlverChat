export const APP_NAME = "AlverChat";
export type Peer = { id:string; name:string; initials:string; color:string; online:boolean; status:string; preview:string; time:string; unread?:number; group?:boolean; typing?:boolean; muted?:boolean; pinned?:boolean };
export type ChatMessage = { id:string; sender:string; text:string; time:string; timestampMs?:number; mine?:boolean; read?:boolean; deliveryStatus?:'pending'|'delivered'; kind?:'text'|'file'|'event'|'deleted'; fileName?:string; edited?:boolean; replyTo?:string };
export type OverlaySnapshot = { peers:Peer[]; unreadTotal:number; open:boolean; dockSide:'left'|'right'; snapToEdge:boolean; bubblesEnabled:boolean; bubbleAutoHideSeconds:number };

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
export type ConnectionReport = { listening:boolean; port:number; peers:number; onlinePeers:number; adapters:string[]; discoveryPort:number; warnings:string[] };
export type BackupResult = { path:string; bytes:number; createdMs:number };
export type ExportResult = { path:string; messages:number };

export interface PlatformBridge {
  listPeers(): Promise<Peer[]>;
  listMessages(chatId:string): Promise<ChatMessage[]>;
  sendMessage(chatId:string,text:string): Promise<ChatMessage>;
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
  editMessageLocal(messageId:string,text:string): Promise<boolean>;
  getMyIdentity(): Promise<{ peerId: string; fingerprint: string }>;
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
  setMutedChat(chatId:string,muted:boolean): Promise<string[]>;
  listGroups(): Promise<Group[]>;
  createGroup(name:string, memberIds:string[], icon?:string): Promise<Group>;
  updateGroup(id:string, patch:GroupPatch): Promise<Group>;
  addGroupMembers(id:string, memberIds:string[]): Promise<Group>;
  removeGroupMember(id:string, peerId:string): Promise<Group>;
  leaveGroup(id:string): Promise<void>;
  /** Stores a 256 px, EXIF-stripped JPEG for my peer card; `null` removes it. */
  setAvatar(avatarDataUrl:string|null): Promise<string|null>;
  getAvatar(): Promise<string|null>;
}

export const DEFAULT_SETTINGS: AppSettings = {
  theme:'system', language:'en', launchAtLogin:false, displayName:'', avatarColor:'violet',
  presence:'Available', customStatus:'', bubblesEnabled:true, idleIcon:'logo',
  iconMorphEnabled:true, iconMorphDelaySeconds:5, bubbleAutoHideSeconds:7,
  screenCorner:'right-center', opacityPercent:90, snapToEdge:true,
  hotkey:'Ctrl+Alt+S', notificationSound:true, notificationsMuted:false, dndEnabled:true,
  dndDays:['Mon','Tue','Wed','Thu','Fri'], dndFrom:'22:00', dndTo:'07:00', mutedChats:[],
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
 async overlaySnapshot(){return {peers:[],unreadTotal:0,open:false,dockSide:'right',snapToEdge:true,bubblesEnabled:true,bubbleAutoHideSeconds:7}},
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
 async runConnectionTest(){return {listening:false,port:0,peers:0,onlinePeers:0,adapters:[],discoveryPort:47986,warnings:['Running outside the desktop app.']}},
 async createBackup(){return unavailable('Backups')},
 async listBackups(){return []},
 async restoreBackup(){return unavailable('Restore')},
 async exportData(){return unavailable('Export')},
 async revealPath(){/* browser preview */},
 async setMutedChat(_chatId,muted){return muted?['muted']:[]},
 async listGroups(){return []},
 async createGroup(){return unavailable('Groups')},
 async updateGroup(){return unavailable('Groups')},
 async addGroupMembers(){return unavailable('Groups')},
 async removeGroupMember(){return unavailable('Groups')},
 async leaveGroup(){return unavailable('Groups')},
 async setAvatar(){return null},
 async getAvatar(){return null},
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
