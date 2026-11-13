import { useEffect, useRef, useState, type PointerEvent } from 'react';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { MessageCircle, Reply, Search, Send, UsersRound, X, ChevronRight } from 'lucide-react';
import { bridgeForRuntime, mockBridge, type OverlaySnapshot, type Peer, type PlatformBridge } from '../../../bridge/src';

function Avatar({ peer }: { peer: Peer }) {
  return <span className={`avatar ${peer.color} ${peer.group ? 'group' : ''}`}>{peer.group ? <UsersRound size={17}/> : peer.initials}<i className={peer.online ? 'online' : ''}/></span>;
}

function useOverlayState() {
  const [bridge, setBridge] = useState<PlatformBridge>(mockBridge);
  const [state, setState] = useState<OverlaySnapshot>({ peers: [], unreadTotal: 0, open: false });
  useEffect(() => { let unlisten: (() => void) | undefined; let mounted = true;
    void bridgeForRuntime().then(async next => { if (!mounted) return; setBridge(next); setState(await next.overlaySnapshot());
      unlisten = await listen<OverlaySnapshot>('overlay-state', event => setState(event.payload));
    });
    return () => { mounted = false; unlisten?.(); };
  }, []);
  return { bridge, state, setState };
}

export function DockWindow() {
  const { bridge, state } = useOverlayState();
  const [draft, setDraft] = useState(''); const [replyId, setReplyId] = useState<string | null>(null);
  const dragTimer = useRef<number | undefined>(); const dragged = useRef(false);
  const unread = state.peers.filter(peer => peer.unread).slice(0, 3);
  useEffect(() => { void bridge.resizeDock(unread.length > 0); }, [bridge, unread.length]);
  const iconPeer = unread[0] ?? state.peers[0];
  const onPointer = (event: PointerEvent) => { const node = event.target as HTMLElement; void bridge.setOverlayHitRegions(Boolean(node.closest('.overlay-hit'))); };
  return <div className="overlay-canvas dock-canvas" onPointerMove={onPointer} onPointerLeave={() => void bridge.setOverlayHitRegions(false)}>
    <div className="dock-stack">
      <div className={`dock-bubbles overlay-hit ${unread.length ? 'has-bubbles' : ''}`}>{unread.map(peer => <article className="dock-bubble" key={peer.id}>
        <button className="bubble-open" onClick={() => void bridge.openFullApp(peer.id)}><Avatar peer={peer}/><span className="dock-bubble-copy"><b>{peer.name}</b><small>{peer.preview}</small></span><time>{peer.time}</time></button>
        <button className="bubble-reply" aria-label={`Quick reply to ${peer.name}`} onClick={() => setReplyId(replyId === peer.id ? null : peer.id)}><Reply size={14}/></button>
        {replyId === peer.id && <form className="quick-reply" onSubmit={event => { event.preventDefault(); if (draft.trim()) void bridge.sendMessage(peer.id, draft.trim()).then(() => { setDraft(''); setReplyId(null); }); }}><input autoFocus value={draft} onChange={event => setDraft(event.target.value)} placeholder="Reply..."/><button><Send size={14}/></button></form>}
      </article>)}</div>
      <button className={`floating-dock dock-trigger overlay-hit ${state.unreadTotal ? 'has-unread' : ''}`} onPointerDown={() => { dragged.current = false; dragTimer.current = window.setTimeout(() => { dragged.current = true; void bridge.startOverlayDrag(); }, 300); }} onPointerUp={() => window.clearTimeout(dragTimer.current)} onPointerCancel={() => window.clearTimeout(dragTimer.current)} onContextMenu={event => { event.preventDefault(); void bridge.openDockMenu(); }} onClick={() => { if (dragged.current) { dragged.current = false; return; } void bridge.toggleCompact(); }} aria-label="Open AlverChat quick chats">{iconPeer && state.unreadTotal ? <Avatar peer={iconPeer}/> : <MessageCircle size={23} fill="currentColor"/>}{state.unreadTotal > 0 && <span>{state.unreadTotal}</span>}</button>
    </div>
  </div>;
}

export function CompactWindow() {
  const { bridge, state } = useOverlayState(); const [query, setQuery] = useState('');
  useEffect(() => { const nativeWindow = getCurrentWindow(); const unlisten = nativeWindow.onFocusChanged(({ payload }) => { if (!payload) void bridge.hideCompact(); });
    const esc = (event: KeyboardEvent) => { if (event.key === 'Escape') void bridge.hideCompact(); }; globalThis.window.addEventListener('keydown', esc);
    return () => { void unlisten.then(fn => fn()); globalThis.window.removeEventListener('keydown', esc); };
  }, [bridge]);
  return <div className="overlay-canvas compact-canvas" onPointerMove={event => void bridge.setOverlayHitRegions(Boolean((event.target as HTMLElement).closest('.overlay-hit')))} onPointerLeave={() => void bridge.setOverlayHitRegions(false)}>
    <section className="compact-drawer overlay-hit"><header><div className="brandmark mini-brand"><MessageCircle size={18}/></div><b>AlverChat</b><span>Quick chats</span><button className="iconbtn" onClick={() => void bridge.hideCompact()} aria-label="Close quick chats"><X size={17}/></button></header>
      <label className="compact-search"><Search size={15}/><input placeholder="Search chats, people..." value={query} onChange={event => setQuery(event.target.value)}/></label>
      <div className="compact-tabs"><button className="selected">Chats</button><button>Groups</button><button>People</button><button>Calls</button></div>
      <div className="compact-list">{state.peers.filter(peer => `${peer.name} ${peer.preview}`.toLowerCase().includes(query.toLowerCase())).map(peer => <button key={peer.id} className="compact-row" onClick={() => void bridge.openFullApp(peer.id)}><Avatar peer={peer}/><span><b>{peer.name}</b><small>{peer.preview}</small></span>{peer.unread && <i>{peer.unread}</i>}</button>)}</div>
      <button className="compact-expand" onClick={() => void bridge.openFullApp()} >Open full AlverChat <ChevronRight size={15}/></button>
    </section>
  </div>;
}

export function MenuWindow() { const { bridge } = useOverlayState(); return <div className="overlay-canvas menu-canvas" onPointerMove={event => void bridge.setOverlayHitRegions(Boolean((event.target as HTMLElement).closest('.overlay-hit')))} onPointerLeave={() => void bridge.setOverlayHitRegions(false)}><section className="overlay-menu overlay-hit"><button onClick={() => { void bridge.hideMenu(); void bridge.toggleCompact(); }}>Open Quick chats</button><button onClick={() => void bridge.openFullApp()}>Open AlverChat</button><button onClick={() => void bridge.quit()}>Quit</button></section></div>; }
