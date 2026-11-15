import { useCallback, useEffect, useMemo, useRef, useState, type PointerEvent as ReactPointerEvent } from 'react';
import { listen } from '@tauri-apps/api/event';
import { Check, ContactRound, Maximize2, Megaphone, MessageCircle, Minus, Phone, Reply, Send, UsersRound, X } from 'lucide-react';
import {
  APP_NAME,
  bridgeForRuntime,
  emptyBrowserBridge,
  type Group,
  type IncomingCallSignal,
  type OverlaySnapshot,
  type Peer,
  type PlatformBridge,
} from '../../../bridge/src';
import { ToastViewport, type ToastView } from '../components';
import { ns, registerNamespace } from '../i18n';
import shellCatalog from '../i18n/en/shell';
import { groupPeer, rosterRows, summaryOf, type GroupThread } from '../groupChats';
import { CallOverlay } from '../calls/CallOverlay';
import { useCall } from '../calls/useCall';
import { needsOnboarding, useAppBridge, type ScreenProps, type ToastTone, type View } from './shared';
import { callClockLabel, dockIconMood, dockOpacity, dockTooltip, idlePhotoFor, morphRevertMs, oldestFirst, peerTimeLabel, showMorphedIcon } from './dock';
import { dockPriority, shouldShowBubble, visibleBubbles } from '../state';

registerNamespace('shell', shellCatalog);
const text = ns('shell');
import { ChatList } from './ChatList';
import { Conversation } from './Conversation';
import { GroupsScreen } from './GroupsScreen';
import { PeopleScreen } from './PeopleScreen';
import { CallsScreen } from './CallsScreen';

function Avatar({ peer }: { peer: Peer }) {
  return <span className={`avatar ${peer.color} ${peer.group ? 'group' : ''}`}>{peer.group ? <UsersRound size={17}/> : peer.initials}<i className={peer.online ? 'online' : ''}/></span>;
}

/** Two short tones. Windows raises no toast for a message (the bubble is the alert), and a
    bubble has no sound of its own, so this keeps the "notification sound" switch honest. */
function chime(): void {
  const AudioCtor = window.AudioContext ?? (window as unknown as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext;
  if (!AudioCtor) return;
  const context = new AudioCtor();
  [880, 1174.66].forEach((frequency, index) => {
    const oscillator = context.createOscillator();
    const gain = context.createGain();
    const at = context.currentTime + index * 0.22;
    oscillator.frequency.value = frequency;
    oscillator.type = 'sine';
    gain.gain.setValueAtTime(0, at);
    gain.gain.linearRampToValueAtTime(0.1, at + 0.02);
    gain.gain.linearRampToValueAtTime(0, at + 0.2);
    oscillator.connect(gain).connect(context.destination);
    oscillator.start(at);
    oscillator.stop(at + 0.22);
  });
  window.setTimeout(() => void context.close(), 900);
}

function useOverlayState() {
  const [bridge, setBridge] = useState<PlatformBridge>(emptyBrowserBridge);
  const [state, setState] = useState<OverlaySnapshot>({ peers: [], unreadTotal: 0, open: false, dockSide: 'right', snapToEdge: true, bubblesEnabled: true, bubbleAutoHideSeconds: 8, showMessagePreview: true, opacityPercent: 90, idleIcon: 'logo', iconMorphEnabled: true, iconMorphDelaySeconds: 5, notificationsQuiet: false, liveCall: null, announcements: [] });
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
    const hasHit = !canceled && Boolean((event?.target as HTMLElement | undefined)?.closest('.overlay-hit'));
    if (wasDrag) { suppressClick.current = true; window.setTimeout(() => { suppressClick.current=false; }, 300); void bridge.endOverlayDrag(hasHit, window.matchMedia('(prefers-reduced-motion: reduce)').matches); }
    else void bridge.setOverlayPointerCapture(false, hasHit);
  }, [bridge, flush, onClick]);
  useEffect(() => { const blur = () => finish(undefined, true); window.addEventListener('blur', blur); return () => window.removeEventListener('blur', blur); }, [finish]);
  const onPointerDown = (event: ReactPointerEvent<HTMLElement>) => {
    if (event.button !== 0 || (event.target as HTMLElement).closest('.no-drag')) return;
    event.preventDefault(); event.currentTarget.setPointerCapture(event.pointerId);
    active.current = { id:event.pointerId, x:event.screenX, y:event.screenY, lastX:event.screenX, lastY:event.screenY, moved:false, frame:null, dx:0, dy:0, target:event.currentTarget };
    void bridge.setOverlayPointerCapture(true, true);
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
  const [bubblesExpired, setBubblesExpired] = useState(false);
  const [iconReverted, setIconReverted] = useState(false);
  /* Bubbles are a visual alert, so the quiet rule that stops the chime stops them too. */
  const bubblesEnabled = state.bubblesEnabled && shouldShowBubble(state.open, state.notificationsQuiet);
  const bubbleHideDelay = state.bubbleAutoHideSeconds;
  const unread = oldestFirst(state.peers.filter(peer => peer.unread));
  const visibleUnread = bubblesExpired ? [] : visibleBubbles(unread, 3);
  const notificationKey = visibleUnread.map(peer => `${peer.id}:${peer.preview}:${peerTimeLabel(peer)}`).join('|');
  useEffect(() => {
    setBubblesExpired(false);
    if (!notificationKey || !bubblesEnabled) return;
    const timer = window.setTimeout(() => setBubblesExpired(true), bubbleHideDelay * 1000);
    return () => window.clearTimeout(timer);
  }, [notificationKey, bubblesEnabled, bubbleHideDelay]);
  /* Settings → Revert delay: the sender's face is a temporary guest on the icon, and a
     fresh unread starts the clock again. The call face is not on this timer. */
  useEffect(() => {
    setIconReverted(false);
    if (!notificationKey || !state.iconMorphEnabled) return;
    const timer = window.setTimeout(() => setIconReverted(true), morphRevertMs(state.iconMorphDelaySeconds));
    return () => window.clearTimeout(timer);
  }, [notificationKey, state.iconMorphEnabled, state.iconMorphDelaySeconds]);
  /* An unacknowledged announcement stays on the icon until it is acknowledged:
     it is a network notice, not a chat preview, so the bubble timer does not hide it. */
  const announcement = state.announcements[0] ?? null;
  const [acking, setAcking] = useState(false);
  const acknowledge = (id: string) => {
    setAcking(true);
    void bridge.acknowledgeAnnouncement(id).catch(() => undefined).finally(() => setAcking(false));
  };
  const bubbles = bubblesEnabled ? visibleUnread : [];
  useEffect(() => { void bridge.resizeDock(bubbles.length > 0 || Boolean(announcement)); }, [bridge, bubbles.length, announcement]);
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    void listen<boolean>('message-alert', event => { if (event.payload) chime(); }).then(next => { unlisten = next; });
    return () => unlisten?.();
  }, []);
  const drag=useNativeDrag(bridge,()=>void bridge.toggleCompact());
  const liveCall = state.liveCall;
  const priority = dockPriority(liveCall !== null, state.unreadTotal);
  const iconPeer = priority === 'call'
    ? state.peers.find(peer => peer.id === liveCall?.chatId)
    : priority === 'unread' ? unread[unread.length - 1] : undefined;
  /* "Icon morph (on activity)" is the only thing that lets a chat borrow the icon's face. */
  const activeFace = showMorphedIcon(state.iconMorphEnabled, iconReverted, liveCall !== null) ? iconPeer : undefined;
  /* The call timer only runs while a call is connected, so an idle dock never repaints. */
  const [nowMs, setNowMs] = useState(() => Date.now());
  useEffect(() => {
    if (!liveCall?.ongoing) return;
    const timer = window.setInterval(() => setNowMs(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [liveCall?.ongoing]);
  /* "Idle icon: My Profile Photo" can only be answered by the engine, which owns the
     local avatar; nothing has been fetched until the choice is actually made. */
  const [selfPhoto, setSelfPhoto] = useState<string | null>(null);
  useEffect(() => {
    if (state.idleIcon !== 'photo') return;
    let mounted = true;
    void bridge.getAvatar().then(next => { if (mounted) setSelfPhoto(next); }).catch(() => undefined);
    return () => { mounted = false; };
  }, [bridge, state.idleIcon]);
  const idlePhoto = idlePhotoFor(state.idleIcon, priority === 'unread' && activeFace ? state.unreadTotal : 0, selfPhoto);
  const mood = dockIconMood(liveCall !== null, state.notificationsQuiet);
  const tooltip = dockTooltip({
    inCall: liveCall !== null,
    peerName: liveCall?.peerName || iconPeer?.name || '',
    unreadTotal: state.unreadTotal,
    quiet: state.notificationsQuiet,
  });
  const onPointer = (event: ReactPointerEvent) => { if(!drag.dragging) void bridge.setOverlayHitRegions(Boolean((event.target as HTMLElement).closest('.overlay-hit'))); };
  return <div className={`overlay-canvas dock-canvas dock-side-${state.dockSide}`} onPointerMove={onPointer} onPointerLeave={() => { if(!drag.dragging) void bridge.setOverlayHitRegions(false); }}>
    <div className="dock-stack" style={{ opacity: dockOpacity(state.opacityPercent) }}>
      <div className={`dock-bubbles overlay-hit ${bubbles.length || announcement ? 'has-bubbles' : ''}`}>
        {announcement ? <article className="dock-bubble announcement">
          <button className="bubble-open" onClick={() => void bridge.openFullApp(announcement.groupId)}>
            <span className="announcement-badge"><Megaphone size={15}/></span>
            <span className="dock-bubble-copy"><b>{announcement.groupName}</b><small>{announcement.text}</small><em>{announcement.authorName}{state.announcements.length > 1 ? <> · {text('announcementMore', '+{{count}} more', { count: state.announcements.length - 1 })}</> : ''}</em></span>
          </button>
          <button className="bubble-reply announcement-ack" aria-label={text('dockAcknowledge', 'Acknowledge announcement')} disabled={acking} onClick={() => acknowledge(announcement.id)}><Check size={14}/></button>
        </article> : null}
        {bubbles.map(peer => <article className="dock-bubble" key={peer.id}>
        <button className="bubble-open" onClick={() => void bridge.openFullApp(peer.id)}><Avatar peer={peer}/><span className="dock-bubble-copy"><b>{peer.name}</b><small>{state.showMessagePreview ? peer.preview : text('dockNewMessage', 'New message')}</small></span><time>{peerTimeLabel(peer)}</time></button>
        <button className="bubble-reply" aria-label={text('dockQuickReplyTo', 'Quick reply to {{name}}', { name: peer.name })} onClick={() => setReplyId(replyId === peer.id ? null : peer.id)}><Reply size={14}/></button>
        {replyId === peer.id && <form className="quick-reply" onSubmit={event => { event.preventDefault(); if (draft.trim() && peer.online) void bridge.sendMessage(peer.id, draft.trim()).then(() => { setDraft(''); setReplyId(null); }).catch(() => {}); }}><input autoFocus disabled={!peer.online} value={draft} onChange={event => setDraft(event.target.value)} placeholder={peer.online ? text('dockReplyHint', 'Reply...') : text('dockPeerUnavailable', 'Peer unavailable on the local network')}/><button disabled={!peer.online}><Send size={14}/></button></form>}
      </article>)}</div>
      <button {...drag.handlers} title={tooltip} className={`floating-dock dock-trigger overlay-hit dock-mood-${mood} ${state.unreadTotal ? 'has-unread' : ''} ${drag.dragging?'is-dragging':''}`} onContextMenu={event => { event.preventDefault(); void bridge.openDockMenu(); }} aria-label={text('dockOpenQuickChats', 'Open AlverChat quick chats')}>{activeFace ? <Avatar peer={activeFace}/> : idlePhoto ? <img alt="" className="dock-idle-photo" src={idlePhoto}/> : <MessageCircle size={23} fill="currentColor"/>}{state.unreadTotal > 0 && !liveCall && <span className="dock-badge">{state.unreadTotal}</span>}{liveCall ? <span className={`dock-state-chip ${mood}`}><Phone size={10}/>{liveCall.ongoing ? callClockLabel(liveCall.startedMs, nowMs) : null}</span> : null}{state.notificationsQuiet && !liveCall ? <span className="dock-state-chip dnd" aria-hidden="true"><Minus size={11}/></span> : null}</button>
    </div>
  </div>;
}

type CompactTab = 'chats' | 'groups' | 'people' | 'calls';

const COMPACT_TABS: ReadonlyArray<{ id: CompactTab; label: string; icon: typeof MessageCircle }> = [
  { id: 'chats', label: 'Chats', icon: MessageCircle },
  { id: 'groups', label: 'Groups', icon: UsersRound },
  { id: 'people', label: 'People', icon: ContactRound },
  { id: 'calls', label: 'Calls', icon: Phone },
];

function prefersDark(): boolean {
  return globalThis.window?.matchMedia('(prefers-color-scheme: dark)').matches ?? false;
}

/**
 * The quick-chat drawer: a compact version of the main app. Every tab is the
 * real screen (chat list, groups, people, calls) and chats open in place as a
 * slide-in conversation, so almost nothing forces the full window.
 */
export function CompactWindow() {
  const { bridge, settings, setSettings, identity, ready } = useAppBridge();
  const [peers, setPeers] = useState<Peer[]>([]);
  const [groups, setGroups] = useState<Group[]>([]);
  const [threads, setThreads] = useState<Record<string, GroupThread>>({});
  const [tab, setTab] = useState<CompactTab>('chats');
  const [activeChat, setActiveChat] = useState<string | null>(null);
  const [chatOpen, setChatOpen] = useState(false);
  const [revision, setRevision] = useState(0);
  const [groupTick, setGroupTick] = useState(0);
  const [toasts, setToasts] = useState<readonly ToastView[]>([]);
  const [systemDark, setSystemDark] = useState(prefersDark);
  const drag = useNativeDrag(bridge);

  const groupsRef = useRef<Group[]>([]);
  groupsRef.current = groups;
  const activeRef = useRef<string | null>(null);
  activeRef.current = activeChat;

  const toast = useCallback((message: string, tone: ToastTone = 'neutral') => {
    setToasts(current => [...current.slice(-2), { id: `${Date.now()}-${Math.random()}`, message, tone }]);
  }, []);

  const callController = useCall(bridge, settings, toast);

  /* Peer list, traffic, call signalling and settings writes all arrive as desktop events. */
  useEffect(() => {
    let alive = true;
    const stops: Array<() => void> = [];
    const on = <T,>(name: string, handler: (payload: T) => void) => {
      void listen<T>(name, event => {
        if (alive) handler(event.payload);
      }).then(stop => {
        if (alive) stops.push(stop);
        else stop();
      });
    };
    void bridge
      .overlaySnapshot()
      .then(snapshot => {
        if (alive) setPeers(snapshot.peers);
      })
      .catch(() => undefined);
    on<{ peers: Peer[] }>('overlay-state', state => setPeers(state.peers));
    on<string>('messages-changed', chatId => {
      if (chatId === activeRef.current) void bridge.markAsRead(chatId).catch(() => undefined);
      setRevision(current => current + 1);
      if (groupsRef.current.some(group => group.id === chatId)) setGroupTick(current => current + 1);
    });
    on<unknown>('settings-changed', () => {
      void bridge.getSettings().then(setSettings).catch(() => undefined);
    });
    return () => {
      alive = false;
      for (const stop of stops) stop();
    };
  }, [bridge, setSettings]);

  useEffect(() => {
    let alive = true;
    void bridge
      .listGroups()
      .then(list => {
        if (alive) setGroups(list);
      })
      .catch(() => undefined);
    return () => {
      alive = false;
    };
  }, [bridge, revision]);

  /* Group rows in the chat list need a preview, clock and unread count of their own. */
  useEffect(() => {
    let alive = true;
    if (groups.length === 0) {
      setThreads({});
      return;
    }
    void Promise.all(
      groups.map(async group => {
        try {
          return [group.id, summaryOf(await bridge.listMessages(group.id))] as const;
        } catch {
          return [group.id, { preview: '', time: '', unread: 0 }] as const;
        }
      }),
    ).then(entries => {
      if (alive) setThreads(Object.fromEntries(entries));
    });
    return () => {
      alive = false;
    };
  }, [bridge, groups, groupTick]);

  useEffect(() => {
    const query = globalThis.window?.matchMedia('(prefers-color-scheme: dark)');
    if (!query) return;
    const onChange = (event: MediaQueryListEvent) => setSystemDark(event.matches);
    query.addEventListener('change', onChange);
    return () => query.removeEventListener('change', onChange);
  }, []);

  useEffect(() => {
    const esc = (event: KeyboardEvent) => {
      if (event.key === 'Escape') void bridge.hideCompact();
    };
    globalThis.window.addEventListener('keydown', esc);
    return () => globalThis.window.removeEventListener('keydown', esc);
  }, [bridge]);

  const patchSettings = useCallback(
    (patch: Partial<typeof settings>) => {
      setSettings(current => ({ ...current, ...patch }));
      void bridge
        .updateSettings(patch)
        .then(setSettings)
        .catch(error => toast(String(error), 'danger'));
    },
    [bridge, setSettings, toast],
  );

  const openChat = useCallback(
    (chatId: string) => {
      setActiveChat(chatId);
      setChatOpen(true);
      setTab('chats');
      void bridge.markAsRead(chatId).catch(() => undefined);
      if (groupsRef.current.some(group => group.id === chatId)) setGroupTick(current => current + 1);
    },
    [bridge],
  );

  const go = useCallback(
    (view: View, chatId?: string) => {
      if (view === 'chats') {
        setTab('chats');
        if (chatId) openChat(chatId);
      } else if (view === 'groups') {
        setTab('groups');
      } else if (view === 'people') {
        setTab('people');
      } else if (view === 'calls') {
        setTab('calls');
      } else {
        /* Settings, profile and onboarding live only in the full window. */
        void bridge.openFullApp();
      }
    },
    [bridge, openChat],
  );

  const mergedPeers = useMemo(() => {
    /* Overlay peers win: the desktop shell may already list a group chat. */
    const byId = new Map(peers.map(peer => [peer.id, peer]));
    return [
      ...peers,
      ...groups
        .filter(group => !byId.has(group.id))
        .map(group => groupPeer(group, threads[group.id], rosterRows(group, byId))),
    ];
  }, [peers, groups, threads]);

  const nameOf = useCallback(
    (id: string) => mergedPeers.find(peer => peer.id === id)?.name ?? id,
    [mergedPeers],
  );

  /* The quick window answers its own copy of every signalling frame, so an
     incoming call rings here even when the full app is closed. */
  const handleCallSignal = callController.handleSignal;
  useEffect(() => {
    if (typeof window === 'undefined' || !('__TAURI_INTERNALS__' in window)) return;
    let alive = true;
    let stop: (() => void) | null = null;
    void listen<IncomingCallSignal>('call-signal', event => {
      if (alive) void handleCallSignal(event.payload, nameOf(event.payload.chatId));
    }).then(unlisten => {
      if (alive) stop = unlisten;
      else unlisten();
    });
    return () => {
      alive = false;
      stop?.();
    };
  }, [handleCallSignal, nameOf]);

  const dark = settings.theme === 'dark' || (settings.theme === 'system' && systemDark);
  const chat = mergedPeers.find(peer => peer.id === activeChat) ?? null;
  const unreadTotal = mergedPeers.reduce((total, peer) => total + (peer.unread ?? 0), 0);
  const needsSetup = ready && needsOnboarding(settings);

  const props: ScreenProps = {
    bridge,
    view: tab,
    go,
    toast,
    peers: mergedPeers,
    activeChat,
    settings,
    patchSettings,
    refreshAnnouncements: () => setRevision(current => current + 1),
    refreshKey: revision,
    identity,
    onCall: (chatId, media) => {
      void callController.start(chatId, media, nameOf(chatId));
    },
  };

  return (
    <div
      className="overlay-canvas compact-canvas"
      onPointerMove={event => {
        if (!drag.dragging) void bridge.setOverlayHitRegions(Boolean((event.target as HTMLElement).closest('.overlay-hit')));
      }}
      onPointerLeave={() => {
        if (!drag.dragging) void bridge.setOverlayHitRegions(false);
      }}
    >
      <section className={`compact-drawer overlay-hit ${dark ? 'dark' : ''}`}>
        {!chatOpen ? (
          <header {...drag.handlers} className={drag.dragging ? 'is-dragging' : ''}>
            <div className="brandmark mini-brand">
              <MessageCircle size={18} />
            </div>
            <b>{APP_NAME}</b>
            <span className="compact-head-actions no-drag">
              <button aria-label={text('compactExpand', 'Expand to full app')} className="iconbtn" onClick={() => void bridge.openFullApp()}>
                <Maximize2 size={15} />
              </button>
              <button aria-label={text('compactClose', 'Close quick chats')} className="iconbtn" onClick={() => void bridge.hideCompact()}>
                <X size={17} />
              </button>
            </span>
          </header>
        ) : null}

        {!chatOpen ? (
          <nav aria-label={text('compactSections', 'Quick chat sections')} className="compact-tabs no-drag">
            {COMPACT_TABS.map(item => (
              <button
                aria-current={tab === item.id ? 'page' : undefined}
                className={tab === item.id ? 'selected' : ''}
                key={item.id}
                onClick={() => setTab(item.id)}
                type="button"
              >
                <item.icon size={13} />
                {item.label}
                {item.id === 'chats' && unreadTotal > 0 ? <i className="compact-tab-badge">{unreadTotal}</i> : null}
              </button>
            ))}
          </nav>
        ) : null}

        <div className="compact-body">
          {needsSetup ? (
            <div className="compact-setup">
              <div className="brandmark">
                <MessageCircle size={22} />
              </div>
              <b>{text('compactSetupTitle', 'Set up AlverChat to start chatting')}</b>
              <p>{text('compactSetupBody', 'Pick your display name and finish setup in the full app — the quick window picks it up automatically.')}</p>
              <button className="primary" onClick={() => void bridge.openFullApp()} type="button">
                {text('openAppAction', 'Open AlverChat')}
              </button>
            </div>
          ) : tab === 'chats' ? (
            <>
              <ChatList
                active={activeChat}
                bridge={bridge}
                onNewChat={() => setTab('people')}
                onNewGroup={() => setTab('groups')}
                onOpenNetwork={() => void bridge.openFullApp()}
                onPatchSettings={patchSettings}
                onSelect={openChat}
                peers={mergedPeers}
                settings={settings}
                toast={toast}
              />
              {chatOpen && chat ? (
                <div className="compact-chat-layer">
                  <Conversation
                    bridge={bridge}
                    chatId={chat.id}
                    detailsOpen={false}
                    key={chat.id}
                    myId={identity?.peerId ?? 'me'}
                    onBack={() => setChatOpen(false)}
                    onCall={kind => props.onCall?.(chat.id, kind === 'voice' ? 'audio' : 'video')}
                    onToggleDetails={() => void bridge.openFullApp(chat.id)}
                    peers={mergedPeers}
                    peer={chat}
                    revision={revision}
                    settings={settings}
                    toast={toast}
                  />
                </div>
              ) : null}
            </>
          ) : tab === 'groups' ? (
            <div className="compact-aux">
              <GroupsScreen {...props} />
            </div>
          ) : tab === 'people' ? (
            <div className="compact-aux">
              <PeopleScreen {...props} />
            </div>
          ) : (
            <div className="compact-aux">
              <CallsScreen {...props} />
            </div>
          )}
        </div>

        <ToastViewport onDismiss={id => setToasts(current => current.filter(entry => entry.id !== id))} toasts={toasts} />
      </section>
      <span className="overlay-hit compact-call-surface">
        <CallOverlay controller={callController} settings={settings} />
      </span>
    </div>
  );
}

export function MenuWindow() { const { bridge } = useOverlayState(); return <div className="overlay-canvas menu-canvas" onPointerMove={event => void bridge.setOverlayHitRegions(Boolean((event.target as HTMLElement).closest('.overlay-hit')))} onPointerLeave={() => void bridge.setOverlayHitRegions(false)}><section className="overlay-menu overlay-hit"><button onClick={() => { void bridge.hideMenu(); void bridge.toggleCompact(); }}>{text('menuOpenQuickChats', 'Open Quick chats')}</button><button onClick={() => void bridge.openFullApp()}>{text('openAppAction', 'Open AlverChat')}</button><button onClick={() => void bridge.resetOverlayPosition()}>{text('menuResetPosition', 'Reset icon position')}</button><button onClick={() => void bridge.quit()}>{text('menuQuit', 'Quit')}</button></section></div>; }
