export type DockPriority = 'call' | 'unread' | 'idle';
export type DockBubble = { id:string; sender:string; text:string; sentAt:number };
export function visibleBubbles(items:DockBubble[],limit=3){return items.slice(-limit)}
export function dockPriority(inCall:boolean,unreadCount:number):DockPriority{return inCall?'call':unreadCount>0?'unread':'idle'}
export function shouldShowBubble(chatOpenAndFocused:boolean,muted:boolean,dnd:boolean){return !chatOpenAndFocused&&!muted&&!dnd}
