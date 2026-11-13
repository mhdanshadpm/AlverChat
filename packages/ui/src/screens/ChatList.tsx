import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode, type RefObject } from 'react';
import type { AppSettings, Peer, PlatformBridge, SearchHit } from '../../../bridge/src';
import {
  AVATAR_PALETTE,
  Avatar,
  Badge,
  ContextMenu,
  EmptyState,
  Icon,
  IconButton,
  ListRow,
  SearchInput,
  Tooltip,
  avatarPaletteFor,
  cx,
  initialsFor,
  type AvatarPaletteColor,
  type ContextMenuEntry,
} from '../components';
import { ns, registerNamespace } from '../i18n';
import chatCatalog from '../i18n/en/chat';
import type { ToastTone } from './shared';
import './chat.css';

registerNamespace('chat', chatCatalog);
const text = ns('chat');

export type ChatFilter = 'all' | 'unread' | 'groups' | 'muted';

export type ChatListProps = {
  bridge: PlatformBridge;
  peers: Peer[];
  active: string | null;
  settings: AppSettings;
  onSelect: (chatId: string) => void;
  onNewChat: () => void;
  onNewGroup: () => void;
  /** Opens the settings view on the network section. */
  onOpenNetwork: () => void;
  onPatchSettings: (patch: Partial<AppSettings>) => void;
  toast: (message: string, tone?: ToastTone) => void;
  /** App owns the global ⌘K shortcut and hands the field ref down. */
  searchRef?: RefObject<HTMLInputElement>;
  className?: string;
};

function presenceFor(peer: Peer) {
  if (!peer.online) return 'offline' as const;
  return /meeting|busy|dnd|do not disturb/i.test(peer.status) ? ('busy' as const) : ('online' as const);
}

/** The desktop shell sends a palette name; anything else falls back to the hash. */
function paletteFor(value: string | undefined, seed: string): AvatarPaletteColor {
  const named = value as AvatarPaletteColor | undefined;
  return named && AVATAR_PALETTE.includes(named) ? named : avatarPaletteFor(seed);
}

function previewFor(peer: Peer): ReactNode {
  if (peer.typing) return <span className="ac-chat-typing">{text('typingNow', 'Typing…')}</span>;
  if (peer.preview) return peer.preview;
  return <span className="ac-chat-preview-empty">{peer.status || text('tapToOpen', 'Open chat')}</span>;
}

function clockOf(ms: number): string {
  return new Date(ms).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });
}

export function ChatList({
  bridge,
  peers,
  active,
  settings,
  onSelect,
  onNewChat,
  onNewGroup,
  onOpenNetwork,
  onPatchSettings,
  toast,
  searchRef,
  className,
}: ChatListProps) {
  const [query, setQuery] = useState('');
  const [filter, setFilter] = useState<ChatFilter>('all');
  const [hits, setHits] = useState<SearchHit[]>([]);
  const [menu, setMenu] = useState<{ peer: Peer; x: number; y: number } | null>(null);
  const [confirm, setConfirm] = useState<{ peer: Peer; kind: 'clear' | 'delete' } | null>(null);
  const localSearch = useRef<HTMLInputElement>(null);

  const pinned = useMemo(() => new Set(settings.pinnedChats), [settings.pinnedChats]);
  const muted = useMemo(() => new Set(settings.mutedChats), [settings.mutedChats]);

  // Message search is debounced: a large store must not be scanned per keystroke.
  useEffect(() => {
    const needle = query.trim();
    if (needle.length < 2) {
      setHits([]);
      return;
    }
    const timer = window.setTimeout(() => {
      void bridge
        .searchMessages(needle)
        .then(setHits)
        .catch(() => setHits([]));
    }, 180);
    return () => window.clearTimeout(timer);
  }, [bridge, query]);

  const isPinned = useCallback((peer: Peer) => pinned.has(peer.id) || Boolean(peer.pinned), [pinned]);
  const isMuted = useCallback((peer: Peer) => muted.has(peer.id) || Boolean(peer.muted), [muted]);

  const visible = useMemo(() => {
    const needle = query.trim().toLowerCase();
    return peers
      .filter(peer => {
        if (filter === 'unread' && !peer.unread) return false;
        if (filter === 'groups' && !peer.group) return false;
        if (filter === 'muted' && !isMuted(peer)) return false;
        if (!needle) return true;
        return `${peer.name} ${peer.preview}`.toLowerCase().includes(needle);
      })
      .sort((left, right) => Number(isPinned(right)) - Number(isPinned(left)));
  }, [peers, query, filter, isPinned, isMuted]);

  const items = useCallback(
    (peer: Peer): ContextMenuEntry[] => [
      { kind: 'item', id: 'open', icon: 'messageCircle', label: text('tapToOpen', 'Open chat'), onSelect: () => onSelect(peer.id) },
      {
        kind: 'item',
        id: 'pin',
        icon: isPinned(peer) ? 'pinOff' : 'pin',
        label: isPinned(peer) ? text('unpinChat', 'Unpin chat') : text('pinChat', 'Pin chat'),
        onSelect: () => {
          void bridge
            .setPinnedChat(peer.id, !isPinned(peer))
            .then(ids => onPatchSettings({ pinnedChats: ids }))
            .catch(error => toast(String(error), 'danger'));
        },
      },
      {
        kind: 'item',
        id: 'mute',
        icon: isMuted(peer) ? 'bell' : 'bellOff',
        label: isMuted(peer) ? text('unmuteChat', 'Unmute notifications') : text('muteChat', 'Mute notifications'),
        onSelect: () => {
          void bridge
            .setMutedChat(peer.id, !isMuted(peer))
            .then(ids => onPatchSettings({ mutedChats: ids }))
            .catch(error => toast(String(error), 'danger'));
        },
      },
      {
        kind: 'item',
        id: 'unread',
        icon: 'mail',
        label: text('markUnreadAction', 'Mark as unread'),
        onSelect: () => {
          void bridge
            .markUnread(peer.id)
            .then(() => toast(text('unreadMarked', 'Marked as unread.')))
            .catch(error => toast(String(error), 'danger'));
        },
      },
      { kind: 'separator', id: 'sep' },
      {
        kind: 'item',
        id: 'clear',
        icon: 'rotateCcw',
        label: text('clearHistory', 'Clear chat history'),
        onSelect: () => setConfirm({ peer, kind: 'clear' }),
      },
      {
        kind: 'item',
        id: 'delete',
        icon: 'trash',
        danger: true,
        label: text('deleteChatAction', 'Delete chat'),
        onSelect: () => setConfirm({ peer, kind: 'delete' }),
      },
    ],
    [bridge, isMuted, isPinned, onPatchSettings, onSelect, toast],
  );

  const online = peers.filter(peer => peer.online).length;
  const status = peers.length
    ? text('peersDiscoveredMany', '{{count}} peers found', { count: peers.length })
    : text('searchingLan', 'Searching the local network');
  const pinnedRows = visible.filter(isPinned);
  const plainRows = visible.filter(peer => !isPinned(peer));
  const filters: ReadonlyArray<readonly [ChatFilter, string, number]> = [
    ['all', text('filterAll', 'All'), peers.length],
    ['unread', text('filterUnread', 'Unread'), peers.filter(peer => peer.unread).length],
    ['groups', text('filterGroups', 'Groups'), peers.filter(peer => peer.group).length],
    ['muted', text('filterMuted', 'Muted'), peers.filter(isMuted).length],
  ];

  return (
    <section className={cx('ac-chat-list', className)} aria-label={text('messages', 'Messages')}>
      <header className="ac-chat-list__head">
        <div className="ac-chat-list__title">
          <div>
            <span className="ac-chat-list__eyebrow">{text('workspace', 'Your workspace')}</span>
            <h1>{text('messages', 'Messages')}</h1>
          </div>
          <IconButton
            iconSize={18}
            label={text('newConversation', 'New conversation')}
            name="plus"
            onClick={onNewChat}
            variant="soft"
          />
        </div>

        <SearchInput
          aria-label={text('searchChats', 'Search chats, people and messages…')}
          onChange={setQuery}
          placeholder={text('searchChats', 'Search chats, people and messages…')}
          ref={searchRef ?? localSearch}
          shortcut="⌘K"
          value={query}
        />

        <div aria-label={text('filterGroup', 'Filter chats')} className="ac-chat-list__filters" role="group">
          {filters.map(([value, label, count]) => (
            <button
              aria-pressed={filter === value}
              className={cx('ac-chat-filter', filter === value && 'ac-chat-filter--on')}
              key={value}
              onClick={() => setFilter(value)}
              type="button"
            >
              {label}
              <small>{count}</small>
            </button>
          ))}
        </div>
      </header>

      <div className="ac-chat-list__scroll">
        {visible.length === 0 ? (
          query ? (
            <EmptyState
              compact
              icon="searchX"
              message={text('noMatchesBody', 'Try another name, keyword or message text.')}
              title={text('noMatchesTitle', 'No matches found')}
            />
          ) : (
            <EmptyState
              compact
              icon="wifi"
              message={text('noChatsBody', 'Chats appear once AlverChat finds someone on this local network.')}
              title={text('noChatsTitle', 'No chats yet')}
            />
          )
        ) : (
          <>
            {pinnedRows.length > 0 ? (
              <>
                <div className="ac-chat-section">
                  {text('pinnedSection', 'Pinned')} <span>{pinnedRows.length}</span>
                </div>
                {pinnedRows.map(peer => (
                  <ChatRow
                    active={active === peer.id}
                    key={peer.id}
                    muted={isMuted(peer)}
                    onContextMenu={point => setMenu({ peer, ...point })}
                    onSelect={onSelect}
                    peer={peer}
                  />
                ))}
              </>
            ) : null}
            {plainRows.length > 0 ? (
              <>
                <div className="ac-chat-section">
                  {text('chatsSection', 'Chats')} <span>{plainRows.length}</span>
                </div>
                {plainRows.map(peer => (
                  <ChatRow
                    active={active === peer.id}
                    key={peer.id}
                    muted={isMuted(peer)}
                    onContextMenu={point => setMenu({ peer, ...point })}
                    onSelect={onSelect}
                    peer={peer}
                  />
                ))}
              </>
            ) : null}
          </>
        )}

        {hits.length > 0 ? (
          <>
            <div className="ac-chat-section">
              {text('inMessages', 'In messages')}{' '}
              <span>{text('searchResultsMany', '{{count}} matches', { count: hits.length })}</span>
            </div>
            {hits.slice(0, 20).map(hit => (
              <ListRow
                active={active === hit.chatId}
                className="ac-chat-hit"
                hint={hit.text}
                key={hit.messageId}
                leading={<Avatar initials={initialsFor(hit.chatName)} size={24} />}
                onClick={() => onSelect(hit.chatId)}
                title={hit.chatName}
                trailing={<time>{clockOf(hit.timestampMs)}</time>}
              />
            ))}
          </>
        ) : null}
      </div>

      <footer className="ac-chat-list__foot">
        <button className="ac-chat-new" onClick={onNewChat} type="button">
          <Icon name="plus" size={16} />
          {text('newConversation', 'New conversation')}
        </button>
        <button className="ac-chat-new ac-chat-new--ghost" onClick={onNewGroup} type="button">
          <Icon name="usersRound" size={16} />
          {text('newGroup', 'New group')}
        </button>
        <div className="ac-chat-lan">
          <span className={cx('ac-chat-lan__dot', online > 0 && 'ac-chat-lan__dot--on')} />
          <span>{status}</span>
          <Tooltip content={settings.lanEncryption ? text('encryptionOn', 'End-to-end encrypted on this local network') : text('encryptionOff', 'Not encrypted — set the network passphrase')} delay={200} wrap>
            <button className="ac-chat-lan__action" onClick={onOpenNetwork} type="button">
              <Icon name={settings.lanEncryption ? 'shieldCheck' : 'shield'} size={14} />
              {text('readyCount', '{{count}} ready', { count: online })}
            </button>
          </Tooltip>
        </div>
      </footer>

      <ContextMenu
        anchor={menu ? { x: menu.x, y: menu.y } : null}
        items={menu ? items(menu.peer) : []}
        label={text('moreOptions', 'More options')}
        onClose={() => setMenu(null)}
        open={Boolean(menu)}
      />

      {confirm ? (
        <ConfirmDialog
          body={
            confirm.kind === 'clear'
              ? text('confirmClear', 'Delete every message in this chat on this device?')
              : text('confirmDeleteChat', 'Remove this chat and its history from this device?')
          }
          onCancel={() => setConfirm(null)}
          onConfirm={() => {
            const { peer, kind } = confirm;
            setConfirm(null);
            const call = kind === 'clear' ? bridge.clearChat(peer.id) : bridge.deleteChat(peer.id);
            void call
              .then(() =>
                toast(
                  kind === 'clear'
                    ? text('chatCleared', 'Chat history cleared on this device.')
                    : text('chatDeleted', 'Chat deleted from this device.'),
                ),
              )
              .catch(error => toast(String(error), 'danger'));
          }}
          title={
            confirm.kind === 'clear'
              ? text('clearHistory', 'Clear chat history')
              : text('deleteChatAction', 'Delete chat')
          }
        />
      ) : null}
    </section>
  );
}

/** One chat row: avatar, name + preview, time + unread, pin and mute markers. */
function ChatRow({
  peer,
  active,
  muted,
  onSelect,
  onContextMenu,
}: {
  peer: Peer;
  active: boolean;
  muted: boolean;
  onSelect: (chatId: string) => void;
  onContextMenu: (point: { x: number; y: number }) => void;
}) {
  return (
    <button
      aria-current={active ? 'true' : undefined}
      className={cx('ac-chat-row', active && 'ac-chat-row--on')}
      onClick={() => onSelect(peer.id)}
      onContextMenu={event => {
        event.preventDefault();
        onContextMenu({ x: event.clientX, y: event.clientY });
      }}
      type="button"
    >
      <Avatar
        color={paletteFor(peer.color, peer.name)}
        group={peer.group}
        initials={peer.initials || initialsFor(peer.name)}
        name={peer.name}
        presence={presenceFor(peer)}
        size={40}
      />
      <span className="ac-chat-row__body">
        <span className="ac-chat-row__top">
          <strong>{peer.name}</strong>
          <time>{peer.time}</time>
        </span>
        <span className="ac-chat-row__bottom">
          <span className="ac-chat-row__preview">{previewFor(peer)}</span>
          <span className="ac-chat-row__marks">
            {muted ? <Icon className="ac-chat-mark" name="bellOff" size={13} /> : null}
            {peer.pinned ? <Icon className="ac-chat-mark" name="pin" size={13} /> : null}
            {peer.unread ? (
              <Badge
                count={peer.unread}
                label={text('unreadCount', '{{count}} unread', { count: peer.unread })}
                tone="danger"
              />
            ) : null}
          </span>
        </span>
      </span>
    </button>
  );
}

/** Destructive confirmation used by clear/delete chat; matches the Dialog board. */
function ConfirmDialog({
  title,
  body,
  onCancel,
  onConfirm,
}: {
  title: string;
  body: string;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  return (
    <div aria-label={title} aria-modal="true" className="ac-chat-confirm" role="alertdialog">
      <div className="ac-chat-confirm__card">
        <span className="ac-chat-confirm__icon">
          <Icon name="alertCircle" size={20} />
        </span>
        <h3>{title}</h3>
        <p>{body}</p>
        <div className="ac-chat-confirm__actions">
          <button className="ac-btn ac-btn--secondary" onClick={onCancel} type="button">
            {text('cancel', 'Cancel')}
          </button>
          <button className="ac-btn ac-btn--danger" onClick={onConfirm} type="button">
            {title}
          </button>
        </div>
      </div>
    </div>
  );
}
