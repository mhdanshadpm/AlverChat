import { useCallback, useEffect, useRef, useState, type PointerEvent as ReactPointerEvent } from 'react';
import { listen } from '@tauri-apps/api/event';
import { MessageCircle, Reply, Search, Send, UsersRound, X, ChevronRight } from 'lucide-react';
import { bridgeForRuntime, mockBridge, type OverlaySnapshot, type Peer, type PlatformBridge } from '../../../bridge/src';

function Avatar({ peer }: { peer: Peer }) {
  return <span className={`avatar ${peer.color} ${peer.group ? 'group' : ''}`}>{peer.group ? <UsersRound size={17}/> : peer.initials}<i className={peer.online ? 'online' : ''}/></span>;
}

function useOverlayState() {
  const [bridge, setBridge] = useState<PlatformBridge>(mockBridge);
  const [state, setState] = useState<OverlaySnapshot>({ peers: [], unreadTotal: 0, open: false, dockSide: 'right', snapToEdge: true });
  useEffect(() => { let unlisten: (() => void) | undefined; let mounted = true;
    void bridgeForRuntime().then(async next => { if (!mounted) return; setBridge(next); setState(await next.overlaySnapshot());
      unlisten = await listen<OverlaySnapshot>('overlay-state', event => setState(event.payload));
    });
    return () => { mounted = false; unlisten?.(); };
  }, []);
  return { bridge, state };
}

function useNativeDrag(bridge: PlatformBridge, onClick?: () => void) {
  const [dragging, setDragging] = useState(false);
  const suppressClick = useRef(false);
  const active = useRef<{ id:number; x:number; y:number; lastX:number; lastY:number; moved:boolean; frame:number|null; dx:number; dy:number; target:HTMLElement } | null>(null);
  const flush = useCallback(() => {
    const current = active.current;
    if (!current || !current.moved) return;
    current.frame = null;
    const { dx, dy } = current; current.dx = 0; current.dy = 0;
    if (dx || dy) void bridge.moveOverlayBy(dx, dy);
  }, [bridge]);
  const finish = useCallback((event?: ReactPointerEvent<HTMLElement>, canceled = false) => {
    const current = active.current; if (!current) return;
    if (current.frame !== null) { window.cancelAnimationFrame(current.frame); current.frame = null; flush(); }
    const wasDrag = current.moved;
    if (current.target.hasPointerCapture(current.id)) current.target.releasePointerCapture(current.id);
    active.current = null; setDragging(false);
    if (wasDrag) { suppressClick.current = true; window.setTimeout(() => { suppressClick.current=false; }, 300); void bridge.endOverlayDrag(!canceled && Boolean((event?.target as HTMLElement | undefined)?.closest('.overlay-hit')), window.matchMedia('(prefers-reduced-motion: reduce)').matches); }
  }, [bridge, flush, onClick]);
  useEffect(() => { const blur = () => finish(undefined, true); window.addEventListener('blur', blur); return () => window.removeEventListener('blur', blur); }, [finish]);
  const onPointerDown = (event: ReactPointerEvent<HTMLElement>) => {
    if (event.button !== 0 || (event.target as HTMLElement).closest('.no-drag')) return;
    event.preventDefault(); event.currentTarget.setPointerCapture(event.pointerId);
    active.current = { id:event.pointerId, x:event.screenX, y:event.screenY, lastX:event.screenX, lastY:event.screenY, moved:false, frame:null, dx:0, dy:0, target:event.currentTarget };
    void bridge.setOverlayHitRegions(true);
  };
  const onPointerMove = (event: ReactPointerEvent<HTMLElement>) => {
    const current = active.current; if (!current || current.id !== event.pointerId) return;
    const totalX=event.screenX-current.x, totalY=event.screenY-current.y;
    if (!current.moved && Math.hypot(totalX,totalY) >= 4) {
      current.moved=true; setDragging(true); void bridge.startOverlayDrag();
      current.dx += totalX; current.dy += totalY;
    } else if (current.moved) {
      current.dx += event.screenX-current.lastX; current.dy += event.screenY-current.lastY;
    }
    current.lastX=event.screenX; current.lastY=event.screenY;
    if (current.moved && current.frame===null) current.frame=window.requestAnimationFrame(flush);
  };
  const onPointerUp = (event: ReactPointerEvent<HTMLElement>) => { if (active.current?.id===event.pointerId) finish(event); };
  const onPointerCancel = (event: ReactPointerEvent<HTMLElement>) => { if (active.current?.id===event.pointerId) finish(event,true); };
  const onClickKey = (event: React.MouseEvent<HTMLElement>) => { if (suppressClick.current) { suppressClick.current=false; event.preventDefault(); return; } onClick?.(); };
  return { dragging, handlers:{onPointerDown,onPointerMove,onPointerUp,onPointerCancel,onClick:onClickKey} };
}

export function DockWindow() {
  const { bridge, state } = useOverlayState();
  const [draft, setDraft] = useState(''); const [replyId, setReplyId] = useState<string | null>(null);
  const unread = state.peers.filter(peer => peer.unread).slice(0, 3);
  useEffect(() => { void bridge.resizeDock(unread.length > 0); }, [bridge, unread.length]);
  const drag=useNativeDrag(bridge,()=>void bridge.toggleCompact());
  const iconPeer = unread[0] ?? state.peers[0];
  const onPointer = (event: ReactPointerEvent) => { if(!drag.dragging) void bridge.setOverlayHitRegions(Boolean((event.target as HTMLElement).closest('.overlay-hit'))); };
  return <div className={`overlay-canvas dock-canvas dock-side-${state.dockSide}`} onPointerMove={onPointer} onPointerLeave={() => { if(!drag.dragging) void bridge.setOverlayHitRegions(false); }}>
    <div className="dock-stack">
      <div className={`dock-bubbles overlay-hit ${unread.length ? 'has-bubbles' : ''}`}>{unread.map(peer => <article className="dock-bubble" key={peer.id}>
        <button className="bubble-open" onClick={() => void bridge.openFullApp(peer.id)}><Avatar peer={peer}/><span className="dock-bubble-copy"><b>{peer.name}</b><small>{peer.preview}</small></span><time>{peer.time}</time></button>
        <button className="bubble-reply" aria-label={`Quick reply to ${peer.name}`} onClick={() => setReplyId(replyId === peer.id ? null : peer.id)}><Reply size={14}/></button>
        {replyId === peer.id && <form className="quick-reply" onSubmit={event => { event.preventDefault(); if (draft.trim()) void bridge.sendMessage(peer.id, draft.trim()).then(() => { setDraft(''); setReplyId(null); }); }}><input autoFocus value={draft} onChange={event => setDraft(event.target.value)} placeholder="Reply..."/><button><Send size={14}/></button></form>}
      </article>)}</div>
      <button {...drag.handlers} className={`floating-dock dock-trigger overlay-hit ${state.unreadTotal ? 'has-unread' : ''} ${drag.dragging?'is-dragging':''}`} onContextMenu={event => { event.preventDefault(); void bridge.openDockMenu(); }} aria-label="Open AlverChat quick chats">{iconPeer && state.unreadTotal ? <Avatar peer={iconPeer}/> : <MessageCircle size={23} fill="currentColor"/>}{state.unreadTotal > 0 && <span>{state.unreadTotal}</span>}</button>
    </div>
  </div>;
}

export function CompactWindow() {
  const { bridge, state } = useOverlayState(); const [query, setQuery] = useState('');
  const drag=useNativeDrag(bridge);
  useEffect(() => {
    const esc = (event: KeyboardEvent) => { if (event.key === 'Escape') void bridge.hideCompact(); }; globalThis.window.addEventListener('keydown', esc);
    return () => { globalThis.window.removeEventListener('keydown', esc); };
  }, [bridge]);
  return <div className="overlay-canvas compact-canvas" onPointerMove={event => { if(!drag.dragging) void bridge.setOverlayHitRegions(Boolean((event.target as HTMLElement).closest('.overlay-hit'))); }} onPointerLeave={() => { if(!drag.dragging) void bridge.setOverlayHitRegions(false); }}>
    <section className="compact-drawer overlay-hit"><header {...drag.handlers} className={drag.dragging?'is-dragging':''}><div className="brandmark mini-brand"><MessageCircle size={18}/></div><b>AlverChat</b><span>Quick chats</span><button className="iconbtn no-drag" onClick={() => void bridge.hideCompact()} aria-label="Close quick chats"><X size={17}/></button></header>
      <label className="compact-search no-drag"><Search size={15}/><input placeholder="Search chats, people..." value={query} onChange={event => setQuery(event.target.value)}/></label>
      <div className="compact-tabs no-drag"><button className="selected">Chats</button><button>Groups</button><button>People</button><button>Calls</button></div>
      <div className="compact-list">{state.peers.filter(peer => `${peer.name} ${peer.preview}`.toLowerCase().includes(query.toLowerCase())).map(peer => <button key={peer.id} className="compact-row" onClick={() => void bridge.openFullApp(peer.id)}><Avatar peer={peer}/><span><b>{peer.name}</b><small>{peer.preview}</small></span>{peer.unread && <i>{peer.unread}</i>}</button>)}</div>
      <button className="compact-expand" onClick={() => void bridge.openFullApp()} >Open full AlverChat <ChevronRight size={15}/></button>
    </section>
  </div>;
}

export function MenuWindow() { const { bridge } = useOverlayState(); return <div className="overlay-canvas menu-canvas" onPointerMove={event => void bridge.setOverlayHitRegions(Boolean((event.target as HTMLElement).closest('.overlay-hit')))} onPointerLeave={() => void bridge.setOverlayHitRegions(false)}><section className="overlay-menu overlay-hit"><button onClick={() => { void bridge.hideMenu(); void bridge.toggleCompact(); }}>Open Quick chats</button><button onClick={() => void bridge.openFullApp()}>Open AlverChat</button><button onClick={() => void bridge.resetOverlayPosition()}>Reset icon position</button><button onClick={() => void bridge.quit()}>Quit</button></section></div>; }
