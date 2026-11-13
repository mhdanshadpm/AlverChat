import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import {
  AVATAR_PALETTE,
  Avatar,
  Badge,
  Button,
  ContextMenu,
  ContextMenuTriggerRegion,
  Dialog,
  EmptyState,
  Icon,
  IconButton,
  ListGroup,
  ListRow,
  SearchInput,
  Skeleton,
  Tabs,
  Tooltip,
  avatarPaletteFor,
  contextMenuItems,
  type AvatarPaletteColor,
  type ContextMenuEntry,
  type PresenceState,
} from '../components';
import { ns } from '../i18n';
import type { Peer } from '../../../bridge/src';
import { ScreenHeader, type ScreenProps } from './shared';
import './PeopleScreen.css';

const text = ns('people');

type Filter = 'all' | 'online' | 'offline';

/**
 * `Peer` carries no host field yet (the mDNS beacon has one, the DTO does not),
 * so the device line is read structurally and simply stays hidden until the
 * bridge starts returning it.
 */
type PeerHost = Peer & { host?: string; device?: string };

function deviceLabelFor(peer: Peer): string {
  const withHost = peer as PeerHost;
  return (withHost.host ?? withHost.device ?? '').trim();
}

/** The 8 palette slots are a union; peer colours arrive as free strings. */
function paletteFor(peer: Peer): AvatarPaletteColor {
  const seed = peer.color;
  if ((AVATAR_PALETTE as readonly string[]).includes(seed)) return seed as AvatarPaletteColor;
  return avatarPaletteFor(peer.id || peer.name);
}

function presenceFor(peer: Peer): PresenceState {
  if (!peer.online) return 'offline';
  const status = peer.status.toLowerCase();
  if (status.includes('busy') || status.includes('meeting')) return 'busy';
  if (status.includes('away')) return 'away';
  return 'online';
}

function statusFor(peer: Peer): string {
  if (peer.status.trim().length > 0) return peer.status;
  return peer.online ? text('online', 'Online') : text('offline', 'Offline');
}

/** Apple vs Windows keyboard hint for the search field. */
function searchShortcut(): string {
  const ua = typeof navigator === 'undefined' ? '' : navigator.userAgent;
  return /Mac|iPhone|iPad|iPod/i.test(ua)
    ? text('searchShortcutMac', '⌘ K')
    : text('searchShortcutWin', 'Ctrl K');
}

export function PeopleScreen({ bridge, go, toast, settings, peers: shellPeers }: ScreenProps) {
  const [people, setPeople] = useState<Peer[]>([]);
  const [loading, setLoading] = useState(true);
  const [query, setQuery] = useState('');
  const [filter, setFilter] = useState<Filter>('all');
  const [menu, setMenu] = useState<{ peer: Peer; x: number; y: number } | null>(null);
  const [clearing, setClearing] = useState<Peer | null>(null);
  const searchRef = useRef<HTMLInputElement>(null);
  /** The shell's list is the fallback when discovery is unreachable. */
  const fallback = useRef<Peer[]>(shellPeers);
  fallback.current = shellPeers;

  const load = useCallback(async () => {
    setLoading(true);
    try {
      setPeople(await bridge.listPeers());
    } catch {
      setPeople(fallback.current);
    } finally {
      setLoading(false);
    }
  }, [bridge]);

  useEffect(() => {
    void load();
  }, [load]);

  /* ⌘K / Ctrl+K focuses the people search, per the list-pane shortcut. */
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (!((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'k')) return;
      event.preventDefault();
      searchRef.current?.focus();
      searchRef.current?.select();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);

  const peopleOnly = useMemo(() => people.filter(peer => !peer.group), [people]);
  const onlineCount = useMemo(() => peopleOnly.filter(peer => peer.online).length, [peopleOnly]);
  const offlineCount = peopleOnly.length - onlineCount;

  const visible = useMemo(() => {
    const needle = query.trim().toLowerCase();
    return peopleOnly.filter(peer => {
      if (filter === 'online' && !peer.online) return false;
      if (filter === 'offline' && peer.online) return false;
      if (needle.length === 0) return true;
      return `${peer.name} ${deviceLabelFor(peer)} ${peer.status} ${peer.preview} ${peer.id}`
        .toLowerCase()
        .includes(needle);
    });
  }, [filter, peopleOnly, query]);

  /* ------------------------------------------------------------------ *
   * Actions
   * ------------------------------------------------------------------ */

  const onMessage = useCallback(
    (peer: Peer) => {
      void bridge.openFullApp(peer.id).catch(() => {
        /* The browser preview has no window manager to raise. */
      });
      go('chats', peer.id);
    },
    [bridge, go],
  );

  /**
   * Single seam for calling: swap the body for the real dial-out
   * (`call_offer` over the peer link) when the call engine lands in phase 6.
   */
  const onCall = useCallback(
    (peer: Peer, kind: 'voice' | 'video') => {
      void peer;
      void kind;
      toast(text('callsPhase6', 'Calls land in phase 6.'), 'neutral');
    },
    [toast],
  );

  const onMute = useCallback(
    async (peer: Peer) => {
      const next = !isMuted(peer, settings.mutedChats);
      try {
        await bridge.setMutedChat(peer.id, next);
        setPeople(current => current.map(item => (item.id === peer.id ? { ...item, muted: next } : item)));
        toast(
          next
            ? text('mutedToast', '{{name}} is muted on this device.', { name: peer.name })
            : text('unmutedToast', '{{name}} is unmuted.', { name: peer.name }),
          'neutral',
        );
      } catch {
        toast(text('muteFailed', 'Mute could not be saved.'), 'danger');
      }
    },
    [bridge, settings.mutedChats, toast],
  );

  const onMarkRead = useCallback(
    async (peer: Peer) => {
      try {
        await bridge.markAsRead(peer.id);
        setPeople(current => current.map(item => (item.id === peer.id ? { ...item, unread: undefined } : item)));
        toast(text('readToast', 'Marked as read.'), 'success');
      } catch {
        toast(text('readFailed', 'The chat could not be marked as read.'), 'danger');
      }
    },
    [bridge, toast],
  );

  const menuEntries = useMemo<readonly ContextMenuEntry[]>(() => {
    const peer = menu?.peer;
    if (!peer) return [];
    const muted = isMuted(peer, settings.mutedChats);
    return contextMenuItems([
      {
        id: 'profile',
        label: text('menuProfile', 'View profile'),
        icon: 'user',
        onSelect: () => {
          setMenu(null);
          go('profile');
        },
      },
      {
        id: 'mute',
        label: muted ? text('menuUnmute', 'Unmute chat') : text('menuMute', 'Mute chat'),
        icon: muted ? 'bell' : 'bellOff',
        onSelect: () => {
          setMenu(null);
          void onMute(peer);
        },
      },
      {
        id: 'read',
        label: text('menuRead', 'Mark as read'),
        icon: 'checkCheck',
        shortcut: 'Ctrl+Shift+M',
        disabled: !peer.unread,
        onSelect: () => {
          setMenu(null);
          void onMarkRead(peer);
        },
      },
      {
        id: 'clear',
        label: text('menuClear', 'Clear chat'),
        icon: 'trash',
        danger: true,
        onSelect: () => {
          setMenu(null);
          setClearing(peer);
        },
      },
    ]);
  }, [go, menu, onMarkRead, onMute, settings.mutedChats]);

  /* ------------------------------------------------------------------ */

  return (
    <section className="pp">
      <ScreenHeader
        actions={
          <>
            <Badge tone={onlineCount > 0 ? 'success' : 'neutral'}>
              <Icon name="wifi" size={13} />
              {text('peersOnline', '{{total}} peers online', { total: onlineCount })}
            </Badge>
            <IconButton
              iconSize={17}
              label={text('rescan', 'Search again')}
              name="refreshCw"
              onClick={() => void load()}
              variant="raised"
            />
          </>
        }
        description={text('subtitle', 'Discover and chat with people on this LAN.')}
        eyebrow={text('eyebrow', 'YOUR LOCAL NETWORK')}
        title={text('title', 'People on your network')}
      />

      <div className="pp-toolbar">
        <SearchInput
          aria-label={text('searchLabel', 'Search people')}
          className="pp-search"
          onChange={setQuery}
          onClear={() => setQuery('')}
          placeholder={text('searchPlaceholder', 'Search people, device or name…')}
          ref={searchRef}
          shortcut={searchShortcut()}
          value={query}
        />
        <Tabs
          label={text('filterLabel', 'Filter people')}
          onChange={next => setFilter(next as Filter)}
          tabs={[
            { id: 'all', label: text('filterAll', 'All'), count: peopleOnly.length },
            { id: 'online', label: text('filterOnline', 'Online'), count: onlineCount },
            { id: 'offline', label: text('filterOffline', 'Offline'), count: offlineCount },
          ]}
          value={filter}
          variant="pill"
        />
      </div>

      <div className="pp-count">
        {loading ? (
          <span className="pp-count__scan">
            <Icon name="loader" size={13} spin />
            {text('scanning', 'Scanning…')}
          </span>
        ) : (
          <span>
            {peopleOnly.length === 1
              ? text('discoveredOne', 'Found 1 person on this network')
              : text('discoveredMany', 'Found {{total}} people on this network', {
                  total: peopleOnly.length,
                })}
          </span>
        )}
        <span className="pp-count__hint">{text('countHint', 'Direct messages are end-to-end encrypted.')}</span>
      </div>

      {loading ? <ScanningPanel /> : null}

      {!loading && peopleOnly.length === 0 ? (
        <EmptyState
          actions={
            <>
              <Button icon="refreshCw" onClick={() => void load()}>{text('searchAgain', 'Search again')}</Button>
              <Button onClick={() => go('settings')} variant="secondary">{text('openSettings', 'Open settings')}</Button>
            </>
          }
          icon="contactRound"
          message={text(
            'emptyPeopleMessage',
            'Keep AlverChat open on another device on this local network. Discovery runs automatically every few seconds.',
          )}
          title={text('emptyPeopleTitle', 'No people found on your network')}
        />
      ) : null}

      {!loading && peopleOnly.length > 0 && visible.length === 0 ? (
        <EmptyState
          actions={<Button onClick={() => setQuery('')} variant="secondary">{text('clearSearch', 'Clear Search')}</Button>}
          icon="searchX"
          message={text('emptyResultsMessage', 'We couldn’t find any matches for “{{query}}”.', { query })}
          title={text('emptyResultsTitle', 'No results found')}
        />
      ) : null}

      {!loading && visible.length > 0 ? (
        <ListGroup className="pp-list">
          {visible.map(peer => (
            <ContextMenuTriggerRegion
              key={peer.id}
              onOpen={point => setMenu({ peer, ...point })}
            >
              <ListRow
                className="pp-row"
                hint={
                  <span className="pp-row__hint">
                    {deviceLabelFor(peer).length > 0 ? (
                      <span className="pp-row__device">{deviceLabelFor(peer)}</span>
                    ) : null}
                    <span className={peer.online ? 'pp-row__status pp-row__status--online' : 'pp-row__status'}>
                      {statusFor(peer)}
                    </span>
                  </span>
                }
                leading={
                  <Avatar
                    color={paletteFor(peer)}
                    initials={peer.initials}
                    name={peer.name}
                    presence={presenceFor(peer)}
                    size={56}
                  />
                }
                title={
                  <span className="pp-row__title">
                    <span>{peer.name}</span>
                    {isMuted(peer, settings.mutedChats) ? (
                      <Icon
                        label={text('mutedAria', 'Muted')}
                        name="bellOff"
                        size={13}
                      />
                    ) : null}
                    {peer.typing ? (
                      <em className="pp-row__typing">{text('typing', 'typing…')}</em>
                    ) : null}
                  </span>
                }
                trailing={
                  <>
                    {peer.unread ? <Badge tone="accent" count={peer.unread} /> : null}
                    <Button
                      icon="messageCircle"
                      onClick={() => onMessage(peer)}
                      size="sm"
                      variant="secondary"
                    >
                      {text('message', 'Message')}
                    </Button>
                    <Tooltip content={text('voiceCall', 'Voice call')} side="bottom">
                      <IconButton
                        iconSize={16}
                        label={text('voiceCallAria', 'Start a voice call with {{name}}', { name: peer.name })}
                        name="phoneCall"
                        onClick={() => onCall(peer, 'voice')}
                        size="sm"
                        variant="raised"
                      />
                    </Tooltip>
                    <Tooltip content={text('videoCall', 'Video call')} side="bottom">
                      <IconButton
                        iconSize={17}
                        label={text('videoCallAria', 'Start a video call with {{name}}', { name: peer.name })}
                        name="video"
                        onClick={() => onCall(peer, 'video')}
                        size="sm"
                        variant="raised"
                      />
                    </Tooltip>
                    <IconButton
                      iconSize={17}
                      label={text('moreActions', 'More actions for {{name}}', { name: peer.name })}
                      name="ellipsisVertical"
                      onClick={event => {
                        const rect = event.currentTarget.getBoundingClientRect();
                        setMenu({ peer, x: rect.left, y: rect.bottom + 4 });
                      }}
                      size="sm"
                      variant="plain"
                    />
                  </>
                }
              />
            </ContextMenuTriggerRegion>
          ))}
        </ListGroup>
      ) : null}

      <ContextMenu
        anchor={menu ? { x: menu.x, y: menu.y } : null}
        items={menuEntries}
        label={text('personMenu', 'Person actions')}
        onClose={() => setMenu(null)}
        open={menu !== null}
      />

      <Dialog
        description={text(
          'clearChatBody',
          'This removes the messages and media shared with {{name}} from this device only.',
          { name: clearing?.name ?? '' },
        )}
        footer={
          <>
            <Button onClick={() => setClearing(null)} variant="secondary">
              {text('cancel', 'Cancel')}
            </Button>
            <Button
              onClick={() => {
                const peer = clearing;
                setClearing(null);
                if (!peer) return;
                void (async () => {
                  try {
                    await bridge.clearChat(peer.id);
                    setPeople(current =>
                      current.map(item =>
                        item.id === peer.id ? { ...item, preview: '', unread: undefined } : item,
                      ),
                    );
                    toast(text('clearChatDone', 'Chat with {{name}} cleared on this device.', { name: peer.name }), 'success');
                  } catch {
                    toast(text('clearChatFailed', 'The chat could not be cleared.'), 'danger');
                  }
                })();
              }}
              variant="danger"
            >
              {text('clearChatConfirm', 'Clear chat')}
            </Button>
          </>
        }
        onOpenChange={open => {
          if (!open) setClearing(null);
        }}
        open={clearing !== null}
        size="sm"
        title={text('clearChatTitle', 'Clear this chat?')}
      />
    </section>
  );
}

function isMuted(peer: Peer, mutedChats: readonly string[]): boolean {
  return peer.muted === true || mutedChats.includes(peer.id);
}

/** The discovery animation from board 1.3, plus shimmering placeholder rows. */
function ScanningPanel() {
  return (
    <div className="pp-scan" role="status">
      <span aria-hidden className="pp-scan__radar">
        <span className="pp-scan__ring pp-scan__ring--1" />
        <span className="pp-scan__ring pp-scan__ring--2" />
        <span className="pp-scan__sweep" />
        <Icon name="monitor" size={22} />
      </span>
      <div className="pp-scan__copy">
        <strong>{text('scanningTitle', 'Scanning your network…')}</strong>
        <span>{text('scanningBody', 'Looking for other AlverChat users on your LAN.')}</span>
        <span aria-hidden className="pp-scan__dots">
          <i />
          <i />
          <i />
          <i />
        </span>
        <p className="pp-scan__note">
          {text('scanningNote', 'This usually takes a few seconds. Make sure other users have AlverChat running.')}
        </p>
      </div>
      <div aria-hidden className="pp-scan__rows">
        {[0, 1, 2].map(index => (
          <div className="pp-scan__row" key={index}>
            <Skeleton variant="avatar" />
            <span className="pp-scan__lines">
              <Skeleton variant="title" />
              <Skeleton variant="text" />
            </span>
          </div>
        ))}
      </div>
    </div>
  );
}

export default PeopleScreen;
