export type DockPriority = 'call' | 'unread' | 'idle';
/** The newest `limit` entries of a list ordered oldest first. */
export function visibleBubbles<T>(items:T[],limit=3):T[]{return items.slice(-limit)}
export function dockPriority(inCall:boolean,unreadCount:number):DockPriority{return inCall?'call':unreadCount>0?'unread':'idle'}
/**
 * Bubbles are a visual alert, so the switches that quiet the sound quiet them too:
 * nothing pops while the app is already on screen or the device is quiet. `quiet` is
 * the same rule the shell uses for the Do-Not-Disturb icon (muted or inside the window).
 */
export function shouldShowBubble(chatOpenAndFocused:boolean,quiet:boolean){return !chatOpenAndFocused&&!quiet}
