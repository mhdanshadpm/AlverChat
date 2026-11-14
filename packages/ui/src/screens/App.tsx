import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { APP_NAME, type Announcement, type AppSettings, type Group, type IncomingCallSignal, type Peer } from '../../../bridge/src';
import { Avatar, Button, EmptyState, Icon, ToastViewport, cx, initialsFor, type IconName, type ToastView } from '../components';
import { CallOverlay } from '../calls/CallOverlay';
import { useCall } from '../calls/useCall';
import { announcementView, loadPendingAnnouncements } from '../announcements';
import { groupPeer, summaryOf, type GroupThread } from '../groupChats';
import { ns, registerNamespace } from '../i18n';
import shellCatalog from '../i18n/en/shell';
import { CallsScreen } from './CallsScreen';
import { ChatDetails } from './ChatDetails';
import { ChatList } from './ChatList';
import { Conversation } from './Conversation';
import { GroupsScreen } from './GroupsScreen';
import { OnboardingScreen } from './OnboardingScreen';
import { PeopleScreen } from './PeopleScreen';
import { ProfileScreen } from './ProfileScreen';
import { AnnouncementBanner } from './SystemStates';
import { SettingsScreen } from './SettingsScreen';
import { needsOnboarding, useAppBridge, type ScreenProps, type ToastTone, type View } from './shared';
import './shell.css';

registerNamespace('shell', shellCatalog);
const text = ns('shell');

const NAV: ReadonlyArray<{ view: View; icon: IconName; label: string }> = [
  { view: 'chats', icon: 'messageCircle', label: text('navChats', 'Chats') },
  { view: 'groups', icon: 'usersRound', label: text('navGroups', 'Groups') },
  { view: 'people', icon: 'contactRound', label: text('navPeople', 'People') },
  { view: 'calls', icon: 'phone', label: text('navCalls', 'Calls') },
  { view: 'settings', icon: 'settings', label: text('navSettings', 'Settings') },
];

const isTauri = (): boolean => typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

function prefersDark(): boolean {
  return globalThis.window?.matchMedia('(prefers-color-scheme: dark)').matches ?? false;
}

export default function App() {
  const { bridge, settings, setSettings, identity, ready } = useAppBridge();
  const [peers, setPeers] = useState<Peer[]>([]);
  const [view, setView] = useState<View>('chats');
  const [activeChat, setActiveChat] = useState<string | null>(null);
  const [detailsOpen, setDetailsOpen] = useState(true);
  const [mobileChat, setMobileChat] = useState(false);
  const [revision, setRevision] = useState(0);
  const [toasts, setToasts] = useState<readonly ToastView[]>([]);
  const [avatar, setAvatar] = useState<string | null>(null);
  const [systemDark, setSystemDark] = useState(prefersDark);
  const [announcements, setAnnouncements] = useState<readonly Announcement[]>([]);
  const [announcementTick, setAnnouncementTick] = useState(0);
  const [groups, setGroups] = useState<Group[]>([]);
  const [groupThreads, setGroupThreads] = useState<Record<string, GroupThread>>({});
  const searchRef = useRef<HTMLInputElement>(null);
  const activeRef = useRef<string | null>(null);
  activeRef.current = activeChat;

  const toast = useCallback((message: string, tone: ToastTone = 'neutral') => {
    setToasts(current => [...current.slice(-2), { id: `${Date.now()}-${Math.random()}`, message, tone }]);
  }, []);

  const callController = useCall(bridge, settings, toast);

  const go = useCallback((next: View, chatId?: string) => {
    setView(next);
    if (chatId) {
      setActiveChat(chatId);
      setMobileChat(true);
    }
  }, []);

  const patchSettings = useCallback(
    (patch: Partial<AppSettings>) => {
      setSettings(current => ({ ...current, ...patch }));
      void bridge
        .updateSettings(patch)
        .then(setSettings)
        .catch(error => toast(String(error), 'danger'));
    },
    [bridge, setSettings, toast],
  );

  useEffect(() => {
    void bridge.listPeers().then(setPeers).catch(() => setPeers([]));
    void bridge.getAvatar().then(setAvatar).catch(() => setAvatar(null));
  }, [bridge]);

  // Evaluated once, after settings load: clearing the display name in Settings
  // must not trap the user back in setup.
  const gated = useRef(false);
  useEffect(() => {
    if (!ready || gated.current) return;
    gated.current = true;
    if (needsOnboarding(settings)) setView('onboarding');
  }, [ready, settings]);

  useEffect(() => {
    if (!activeChat && peers.length > 0) setActiveChat(peers[0].id);
  }, [activeChat, peers]);

  /* Desktop events: discovery, tray navigation, new traffic, hotkey failures, settings writes. */
  useEffect(() => {
    if (!isTauri()) return;
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
    on<{ peers: Peer[] }>('overlay-state', state => setPeers(state.peers));
    on<string | null>('navigate-chat', chatId => {
      if (chatId) {
        setActiveChat(chatId);
        setMobileChat(true);
      }
      setView('chats');
    });
    on<string>('messages-changed', chatId => {
      if (chatId === activeRef.current) void bridge.markAsRead(chatId).catch(() => undefined);
      setRevision(current => current + 1);
    });
    on<string>('hotkey-error', combo =>
      toast(text('hotkeyFailed', 'The global shortcut {{combo}} could not be registered.', { combo }), 'danger'),
    );
    on<unknown>('settings-changed', () => {
      void bridge.getSettings().then(setSettings).catch(() => undefined);
    });
    return () => {
      alive = false;
      for (const stop of stops) stop();
    };
  }, [bridge, setSettings, toast]);

  useEffect(() => {
    const query = globalThis.window?.matchMedia('(prefers-color-scheme: dark)');
    if (!query) return;
    const onChange = (event: MediaQueryListEvent) => setSystemDark(event.matches);
    query.addEventListener('change', onChange);
    return () => query.removeEventListener('change', onChange);
  }, []);

  const dark = settings.theme === 'dark' || (settings.theme === 'system' && systemDark);

  /* LAN announcements (spec 7.12): the newest one still owed a tap stays on screen. */
  useEffect(() => {
    if (!ready) return;
    let live = true;
    void loadPendingAnnouncements(bridge, identity?.peerId).then((items) => {
      if (live) setAnnouncements(items);
    });
    return () => {
      live = false;
    };
  }, [announcementTick, bridge, identity?.peerId, ready, revision]);

  /* Groups live beside discovered peers in the chat rail, with a thread summary. */
  useEffect(() => {
    if (!ready) return;
    let live = true;
    void bridge.listGroups()
      .then(list => { if (live) setGroups(list); })
      .catch(() => undefined);
    return () => { live = false; };
  }, [bridge, ready, revision]);

  useEffect(() => {
    if (groups.length === 0) {
      setGroupThreads({});
      return;
    }
    let live = true;
    void Promise.all(groups.map(async (group) => {
      try {
        return [group.id, summaryOf(await bridge.listMessages(group.id))] as const;
      } catch {
        return [group.id, { preview: '', time: '', unread: 0 }] as const;
      }
    })).then((entries) => {
      if (live) setGroupThreads(Object.fromEntries(entries));
    });
    return () => { live = false; };
  }, [bridge, groups, revision]);

  const chatPeers = useMemo(() => {
    const known = new Set(peers.map(peer => peer.id));
    return [
      ...peers,
      ...groups.filter(group => !known.has(group.id)).map(group => groupPeer(group, groupThreads[group.id])),
    ];
  }, [groupThreads, groups, peers]);

  const nameOf = useCallback(
    (id: string) => chatPeers.find((peer) => peer.id === id)?.name ?? id,
    [chatPeers],
  );

  /* Call signalling arrives on the peer session and is answered by the call engine. */
  const handleCallSignal = callController.handleSignal;
  useEffect(() => {
    if (!isTauri()) return;
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

  function acknowledge(item: Announcement) {
    void bridge
      .acknowledgeAnnouncement(item.id)
      .then(() => {
        setAnnouncements((current) => current.filter((entry) => entry.id !== item.id));
        toast(text('announcementAcked', 'Announcement acknowledged'), 'success');
      })
      .catch((error: unknown) => toast(String(error), 'danger'));
  }

  const latestAnnouncement = announcements[0] ?? null;
  const announcementGroup = latestAnnouncement
    ? peers.find((peer) => peer.id === latestAnnouncement.groupId)?.name
    : undefined;
  const pendingView = latestAnnouncement
    ? announcementView(latestAnnouncement, nameOf, identity?.peerId, announcementGroup)
    : null;

  const announcementStrip =
    pendingView && latestAnnouncement ? (
      <div className="app-announce">
        <AnnouncementBanner
          acknowledge={() => acknowledge(latestAnnouncement)}
          announcement={pendingView}
          meta={
            <>
              {announcementGroup ? (
                <>
                  <span>{announcementGroup}</span>
                  <span aria-hidden>·</span>
                </>
              ) : null}
              <span>{pendingView.from}</span>
              <span aria-hidden>·</span>
              <time>{pendingView.at}</time>
              <Button
                icon="messageCircle"
                onClick={() => go('chats', latestAnnouncement.groupId)}
                size="sm"
                variant="ghost"
              >
                {text('openAnnouncementGroup', 'Open group')}
              </Button>
            </>
          }
        />
      </div>
    ) : null;

  /* ⌘K focuses the list search, ⌘W closes details, and the dev keys are dead in release builds. */
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const mod = event.metaKey || event.ctrlKey;
      if (mod && event.key.toLowerCase() === 'k') {
        event.preventDefault();
        setView('chats');
        searchRef.current?.focus();
        return;
      }
      if (mod && event.key.toLowerCase() === 'w') {
        event.preventDefault();
        setDetailsOpen(false);
        return;
      }
      if (!import.meta.env.PROD) return;
      const blocked =
        event.key === 'F5' ||
        event.key === 'F12' ||
        (mod && event.key.toLowerCase() === 'r') ||
        (mod && event.shiftKey && event.key.toLowerCase() === 'i');
      if (blocked) event.preventDefault();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);

  const chat = useMemo(() => chatPeers.find(peer => peer.id === activeChat) ?? null, [chatPeers, activeChat]);
  const unreadTotal = chatPeers.reduce((total, peer) => total + (peer.unread ?? 0), 0);
  const online = peers.filter(peer => peer.online).length;

  const props: ScreenProps = {
    bridge,
    view,
    go,
    toast,
    peers,
    activeChat,
    settings,
    patchSettings,
    refreshAnnouncements: () => setAnnouncementTick((tick) => tick + 1),
    refreshKey: revision,
    identity,
    onCall: (chatId, media) => {
      const peer = chatPeers.find((entry) => entry.id === chatId);
      void callController.start(chatId, media, peer?.name ?? chatId);
    },
  };

  return (
    <main className={cx('app', dark && 'dark', mobileChat && 'mobile-chat')} onContextMenu={event => event.preventDefault()}>
      <aside className="rail">
        <button aria-label={text('openChats', 'Open chats')} className="brandmark" onClick={() => go('chats')} type="button">
          <Icon filled name="messageCircle" size={22} />
          <span className="brand-dot">•••</span>
        </button>
        <button aria-label={text('openMyProfile', 'Open my profile')} className="rail-user" onClick={() => go('profile')} type="button">
          <Avatar
            initials={initialsFor(settings.displayName)}
            name={settings.displayName || APP_NAME}
            size={40}
            src={avatar ?? undefined}
          />
        </button>
        <nav className="rail-nav">
          {NAV.map(item => (
            <button
              aria-current={view === item.view ? 'page' : undefined}
              className={cx(view === item.view && 'selected')}
              key={item.view}
              onClick={() => go(item.view)}
              title={item.label}
              type="button"
            >
              <Icon name={item.icon} size={19} />
              <span>{item.label}</span>
              {item.view === 'chats' && unreadTotal > 0 ? <b className="rail-count">{unreadTotal}</b> : null}
            </button>
          ))}
        </nav>
        <div className="rail-bottom">
          <button
            aria-label={text('toggleTheme', 'Toggle theme')}
            onClick={() => patchSettings({ theme: dark ? 'light' : 'dark' })}
            title={dark ? text('themeDark', 'Dark') : text('themeLight', 'Light')}
            type="button"
          >
            <Icon name={dark ? 'sun' : 'moon'} size={19} />
          </button>
          <span
            className={cx('rail-status-dot', online === 0 && 'rail-status-dot--off')}
            title={online ? text('lanReady', 'LAN discovery is connected') : text('lanSearching', 'LAN discovery is not connected')}
          />
        </div>
      </aside>

      <div className="app-stack">
        {announcementStrip}
        <div className="main-area">
          {view === 'chats' ? (
            <>
              <ChatList
                active={activeChat}
                bridge={bridge}
                onNewChat={() => go('people')}
                onNewGroup={() => go('groups')}
                onOpenNetwork={() => go('settings')}
                onPatchSettings={patchSettings}
                onSelect={id => {
                  setActiveChat(id);
                  setMobileChat(true);
                }}
                peers={chatPeers}
                searchRef={searchRef}
                settings={settings}
                toast={toast}
              />
              {chat ? (
                <Conversation
                  bridge={bridge}
                  chatId={chat.id}
                  detailsOpen={detailsOpen}
                  key={chat.id}
                  myId={identity?.peerId ?? 'me'}
                  onBack={() => setMobileChat(false)}
                  onCall={kind => props.onCall?.(chat.id, kind === 'voice' ? 'audio' : 'video')}
                  onToggleDetails={setDetailsOpen}
                  peers={peers}
                  peer={chat}
                  revision={revision}
                  settings={settings}
                  toast={toast}
                />
              ) : (
                <div className="ac-page__empty">
                  <EmptyState
                    icon="messageCircle"
                    message={text('noChatBody', 'Pick someone from the list, or open People to find a peer on this network.')}
                    title={text('noChatTitle', 'No chat selected')}
                  />
                </div>
              )}
              {detailsOpen && chat ? (
                <ChatDetails
                  bridge={bridge}
                  chatId={chat.id}
                  identity={identity}
                  key={`details-${chat.id}`}
                  onClose={() => setDetailsOpen(false)}
                  onOpenChat={id => {
                    setActiveChat(id);
                    setMobileChat(true);
                  }}
                  onPatchSettings={patchSettings}
                  peers={peers}
                  peer={chat}
                  settings={settings}
                  toast={toast}
                />
              ) : null}
            </>
          ) : (
            <div className="ac-page">
              {view === 'onboarding' ? (
                <OnboardingScreen {...props} />
              ) : view === 'groups' ? (
                <GroupsScreen {...props} />
              ) : view === 'people' ? (
                <PeopleScreen {...props} />
              ) : view === 'calls' ? (
                <CallsScreen {...props} />
              ) : view === 'profile' ? (
                <ProfileScreen {...props} />
              ) : view === 'settings' ? (
                <SettingsScreen {...props} />
              ) : null}
            </div>
          )}
        </div>
      </div>

      <CallOverlay controller={callController} settings={settings} />
      <ToastViewport onDismiss={id => setToasts(current => current.filter(entry => entry.id !== id))} toasts={toasts} />
    </main>
  );
}
