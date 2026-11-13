export const APP_NAME = "AlverChat";
export type Peer = { id:string; name:string; initials:string; color:string; online:boolean; status:string; preview:string; time:string; unread?:number; group?:boolean; typing?:boolean; muted?:boolean; pinned?:boolean };
export type ChatMessage = { id:string; sender:string; text:string; time:string; timestampMs?:number; mine?:boolean; read?:boolean; deliveryStatus?:'pending'|'delivered'; kind?:'text'|'file'|'event'|'deleted'; fileName?:string; edited?:boolean; replyTo?:string };
export type OverlaySnapshot = { peers:Peer[]; unreadTotal:number; open:boolean; dockSide:'left'|'right'; snapToEdge:boolean; bubblesEnabled:boolean; bubbleAutoHideSeconds:number };
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
}

const noBrowserPeers:Peer[] = [];
export const emptyBrowserBridge:PlatformBridge = {
 async listPeers(){return noBrowserPeers},
 async listMessages(){return []},
 async sendMessage(){throw new Error('Messaging is unavailable outside the AlverChat desktop app.')},
 async overlaySnapshot(){return {peers:noBrowserPeers,unreadTotal:0,open:false,dockSide:'right',snapToEdge:true,bubblesEnabled:true,bubbleAutoHideSeconds:8}}
 ,async toggleCompact(){/* browser preview */}
 ,async hideCompact(){/* browser preview */}
 ,async openFullApp(){/* browser preview */}
 ,async setOverlayHitRegions(){/* browser preview */}
 ,async setOverlayPointerCapture(){/* browser preview */}
 ,async startOverlayDrag(){/* browser preview */}
 ,async moveOverlayBy(){/* browser preview */}
 ,async endOverlayDrag(){/* browser preview */}
 ,async resizeDock(){/* browser preview */}
 ,async openDockMenu(){/* browser preview */}
 ,async hideMenu(){/* browser preview */}
 ,async quit(){/* browser preview */}
 ,async resetOverlayPosition(){/* browser preview */}
 ,async setSnapToEdge(){/* browser preview */}
 ,async setBubbleSettings(){/* browser preview */}
 ,async markAsRead(){/* browser preview */}
 ,async deleteMessageLocal(){return true}
 ,async editMessageLocal(){return true}
 ,async getMyIdentity(){return { peerId: 'browser-preview', fingerprint: 'preview-fingerprint' }}
};

export async function bridgeForRuntime():Promise<PlatformBridge>{
  if (typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window) {
    const { tauriBridge } = await import('./tauri');
    return tauriBridge;
  }
  return emptyBrowserBridge;
}
