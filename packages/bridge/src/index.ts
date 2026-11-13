export const APP_NAME = "AlverChat";
export type Peer = { id:string; name:string; initials:string; color:string; online:boolean; status:string; preview:string; time:string; unread?:number; group?:boolean; typing?:boolean; muted?:boolean; pinned?:boolean };
export type ChatMessage = { id:string; sender:string; text:string; time:string; mine?:boolean; read?:boolean; kind?:'text'|'file'|'event'; fileName?:string };
export interface PlatformBridge {
  listPeers(): Promise<Peer[]>;
  listMessages(chatId:string): Promise<ChatMessage[]>;
  sendMessage(chatId:string,text:string): Promise<ChatMessage>;
  setTheme(theme:'light'|'dark'): Promise<void>;
}

const peers:Peer[] = [
 {id:'arjun',name:'Arjun Kumar',initials:'AK',color:'peach',online:true,status:'Available',preview:'Invoice updated. Please check.',time:'11:24 AM',unread:3,pinned:true},
 {id:'billing',name:'Billing Team',initials:'BT',color:'violet',online:true,status:'12 members · 4 online',preview:'Anu: Purchase entry completed.',time:'11:20 AM',unread:5,group:true,pinned:true},
 {id:'fathima',name:'Fathima P',initials:'FP',color:'rose',online:true,status:'Typing…',preview:'Typing…',time:'11:18 AM',typing:true},
 {id:'design',name:'Design Team',initials:'DT',color:'blue',online:true,status:'6 members · 3 online',preview:'You: New poster added.',time:'11:07 AM',group:true,muted:true},
 {id:'sarath',name:'Sarath S',initials:'SS',color:'green',online:true,status:'Available',preview:'Video ready, please review.',time:'10:22 AM',unread:1},
 {id:'support',name:'IT Support',initials:'IT',color:'slate',online:false,status:'Last seen 9:50 AM',preview:'System update at 4 PM.',time:'9:50 AM'},
];
const messages:Record<string,ChatMessage[]> = {
 arjun:[{id:'1',sender:'Arjun Kumar',text:'Invoice updated. Please check.',time:'10:32 AM'},{id:'2',sender:'You',text:"Yes, I'll review and confirm in 10 minutes.",time:'10:35 AM',mine:true,read:true},{id:'3',sender:'Arjun Kumar',text:'Invoice_ML00037.pdf · 2.4 MB',time:'10:48 AM',kind:'file',fileName:'Invoice_ML00037.pdf'},{id:'4',sender:'Arjun Kumar',text:'Also, update the stock report.',time:'11:10 AM'},{id:'5',sender:'You',text:"Noted. I'll update it.",time:'11:24 AM',mine:true,read:true}],
 billing:[{id:'1',sender:'Anu',text:'Purchase entry completed for today. ✅',time:'10:42 AM'},{id:'2',sender:'Sarath',text:'Design for the invoice is ready.',time:'10:56 AM'},{id:'3',sender:'You',text:'Thanks, I’ll review it now.',time:'11:08 AM',mine:true,read:true}],
 fathima:[{id:'1',sender:'Fathima P',text:'Can you share the stock report?',time:'11:18 AM'}], design:[{id:'1',sender:'You',text:'New poster added.',time:'11:07 AM',mine:true}], sarath:[{id:'1',sender:'Sarath S',text:'Video ready, please review.',time:'10:22 AM'}],support:[{id:'1',sender:'IT Support',text:'System update at 4 PM.',time:'9:50 AM'}]
};
export const mockBridge:PlatformBridge = {
 async listPeers(){return peers},
 async listMessages(id){return messages[id] ?? []},
 async sendMessage(id,text){const msg={id:crypto.randomUUID(),sender:'You',text,time:new Date().toLocaleTimeString([], {hour:'numeric',minute:'2-digit'}),mine:true,read:false} satisfies ChatMessage; (messages[id]??=[]).push(msg);return msg},
 async setTheme(){/* mock preference is stored in the UI */}
};

export async function bridgeForRuntime():Promise<PlatformBridge>{
  if (typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window) {
    const { tauriBridge } = await import('./tauri');
    return tauriBridge;
  }
  return mockBridge;
}
