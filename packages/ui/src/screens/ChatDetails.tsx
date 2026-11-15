import { useCallback, useEffect, useMemo, useState } from 'react';
import type { AppSettings, Attachment, ChatMessage, Group, Identity, Peer, PlatformBridge, PinnedMessage } from '../../../bridge/src';
import {
  Badge,
  Button,
  EmptyState,
  Icon,
  IconButton,
  ListRow,
  PeerAvatar,
  Segmented,
  Toggle,
  Tooltip,
  cx,
  formatFileSize,
  initialsFor,
  presenceForPeer,
  statusLineFor,
} from '../components';
import { ns, registerNamespace } from '../i18n';
import chatCatalog from '../i18n/en/chat';
import { useAttachmentPreviews } from '../attachmentPreviews';
import { authorName, messageTime, type ToastTone } from './shared';
import './chat.css';

registerNamespace('chat', chatCatalog);
const text = ns('chat');

/** A stable "nothing to draw", so a closed tab cannot re-run the preview effect. */
const NO_ATTACHMENTS: readonly Attachment[] = [];

type DetailsTab = 'media' | 'files' | 'links' | 'starred' | 'pinned' | 'members';

const LINK = /\bhttps?:\/\/[^\s<>"')]+/gi;

export type ChatDetailsProps = {
  bridge: PlatformBridge;
  peer: Peer | null;
  chatId: string;
  peers: Peer[];
  settings: AppSettings;
  identity: Identity | null;
  onClose: () => void;
  onOpenChat: (chatId: string) => void;
  onPatchSettings: (patch: Partial<AppSettings>) => void;
  toast: (message: string, tone?: ToastTone) => void;
  className?: string;
};

export function ChatDetails({
  bridge,
  peer,
  chatId,
  peers,
  settings,
  identity,
  onClose,
  onOpenChat,
  onPatchSettings,
  toast,
  className,
}: ChatDetailsProps) {
  const [tab, setTab] = useState<DetailsTab>('media');
  const [attachments, setAttachments] = useState<Attachment[]>([]);
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [starred, setStarred] = useState<ChatMessage[]>([]);
  const [pinned, setPinned] = useState<PinnedMessage[]>([]);
  const [group, setGroup] = useState<Group | null>(null);
  const [confirm, setConfirm] = useState<'clear' | 'delete' | 'leave' | null>(null);

  const reload = useCallback(async () => {
    const [files, history, stars, pins, groups] = await Promise.all([
      bridge.attachmentsFor(chatId).catch(() => [] as Attachment[]),
      bridge.listMessages(chatId).catch(() => [] as ChatMessage[]),
      bridge.starredMessages(chatId).catch(() => [] as ChatMessage[]),
      bridge.pinnedMessages(chatId).catch(() => [] as PinnedMessage[]),
      bridge.listGroups().catch(() => [] as Group[]),
    ]);
    setAttachments(files);
    setMessages(history);
    setStarred(stars);
    setPinned(pins);
    setGroup(groups.find(entry => entry.id === chatId) ?? null);
  }, [bridge, chatId]);

  useEffect(() => {
    void reload();
  }, [reload]);

  /** The panel keeps people as ids in its lists; nobody should read one as a name. */
  const nameOf = useCallback(
    (id: string) =>
      authorName(id, peers, {
        id: identity?.peerId ?? 'me',
        name: settings.displayName.trim() || text('groupYou', 'You'),
      }),
    [identity?.peerId, peers, settings.displayName],
  );

  // A tile is a picture, so the Media tab holds the attachments this build can draw one
  // for; a video, a recording and a document all belong in Files with their size.
  const media = useMemo(() => attachments.filter(entry => entry.kind === 'image'), [attachments]);
  const files = useMemo(() => attachments.filter(entry => entry.kind !== 'image'), [attachments]);
  // Only the tab that is open pays for bytes, and only once per attachment.
  const [previewOf] = useAttachmentPreviews(bridge, tab === 'media' ? media : NO_ATTACHMENTS);
  const links = useMemo(() => {
    const found = new Set<string>();
    for (const message of messages) for (const hit of message.text.match(LINK) ?? []) found.add(hit);
    return [...found].slice(0, 40);
  }, [messages]);
  const usedBytes = attachments.reduce((total, entry) => total + entry.bytes, 0);
  const isGroup = Boolean(peer?.group) || group !== null;
  const members = useMemo(() => {
    if (!group) return [];
    return group.members.map((id, index) => ({
      id,
      name: nameOf(id),
      admin: id === group.admin,
      color: (peers.find(entry => entry.id === id)?.color ?? undefined) as string | undefined,
      online: id === 'me' ? true : (peers.find(entry => entry.id === id)?.online ?? index % 2 === 0),
      self: id === 'me' || id === identity?.peerId,
    }));
  }, [group, nameOf, peers, identity]);

  const tabs = [
    { value: 'media', label: text('mediaTab', 'Media') },
    { value: 'files', label: text('filesTab', 'Files') },
    { value: 'links', label: text('linksTab', 'Links') },
    { value: 'starred', label: text('starredTab', 'Starred') },
    { value: 'pinned', label: text('pinnedTab', 'Pinned') },
    ...(isGroup ? [{ value: 'members', label: text('membersTab', 'Members') }] : []),
  ] as const;

  const pinnedIds = new Set(settings.pinnedChats);
  const isPinned = pinnedIds.has(chatId) || Boolean(peer?.pinned);
  const isMuted = settings.mutedChats.includes(chatId) || Boolean(peer?.muted);

  return (
    <aside aria-label={text('detailsTitle', 'Chat details')} className={cx('ac-details', className)}>
      <div className="ac-details__head">
        <span className="ac-chat-list__eyebrow">{text('detailsTitle', 'Chat details')}</span>
        <IconButton iconSize={17} label={text('close', 'Close')} name="x" onClick={onClose} />
      </div>

      <div className="ac-details__identity">
        <PeerAvatar
          group={isGroup}
          initials={peer?.initials || initialsFor(peer?.name ?? chatId)}
          name={peer?.name ?? chatId}
          peerId={isGroup ? null : peer?.id}
          presence={peer ? presenceForPeer(peer) : 'offline'}
          size={56}
          source={bridge}
        />
        <div>
          <h2>{peer?.name ?? chatId}</h2>
          <p>{(peer && statusLineFor(peer)) || text('encryptionOn', 'End-to-end encrypted on this local network')}</p>
        </div>
        {group?.description ? <p>{group.description}</p> : null}
      </div>

      <ListRow
        icon="bellOff"
        title={text('muteToggle', 'Mute notifications')}
        trailing={
          <Toggle
            label={text('muteToggle', 'Mute notifications')}
            checked={isMuted}
            onChange={checked => {
              void bridge
                .setMutedChat(chatId, checked)
                .then(ids => onPatchSettings({ mutedChats: ids }))
                .catch(error => toast(String(error), 'danger'));
            }}
          />
        }
      />
      <ListRow
        icon="pin"
        title={text('pinChat', 'Pin chat')}
        trailing={
          <Toggle
            label={text('pinChat', 'Pin chat')}
            checked={isPinned}
            onChange={checked => {
              void bridge
                .setPinnedChat(chatId, checked)
                .then(ids => onPatchSettings({ pinnedChats: ids }))
                .catch(error => toast(String(error), 'danger'));
            }}
          />
        }
      />

      <Segmented
        label={text('detailsTitle', 'Chat details')}
        onChange={value => setTab(value as DetailsTab)}
        options={tabs.map(entry => ({ label: entry.label, value: entry.value }))}
        scroll
        value={tab}
      />

      {tab === 'media' ? (
        media.length === 0 ? (
          <EmptyState compact icon="image" message={text('emptySharedBody', 'Shared media, files and pinned messages appear here.')} title={text('emptySharedTitle', 'No shared content')} />
        ) : (
          <div className="ac-details__block">
            <h3>{text('mediaTab', 'Media')}</h3>
            <div className="ac-details__grid">
              {media.map(entry => (
                <button
                  className="ac-details__thumb"
                  key={entry.id}
                  onClick={() => void bridge.openAttachment(entry.id).catch(() => undefined)}
                  type="button"
                >
                  {previewOf(entry.id) ? (
                    <img alt={entry.name} src={previewOf(entry.id) ?? ''} />
                  ) : (
                    <Icon name="image" size={20} />
                  )}
                </button>
              ))}
            </div>
          </div>
        )
      ) : null}

      {tab === 'files' ? (
        files.length === 0 ? (
          <EmptyState compact icon="file" message={text('emptySharedBody', 'Shared media, files and pinned messages appear here.')} title={text('emptySharedTitle', 'No shared content')} />
        ) : (
          <div className="ac-details__block">
            <h3>{text('sharedFiles', 'Shared files')}</h3>
            {files.map(entry => (
              <ListRow
                hint={`${formatFileSize(entry.bytes)} · ${entry.state}`}
                icon={entry.kind === 'audio' ? 'fileAudio' : 'file'}
                key={entry.id}
                onClick={() => void bridge.openAttachment(entry.id).catch(() => undefined)}
                title={entry.name}
                trailing={<Icon name="download" size={15} />}
              />
            ))}
          </div>
        )
      ) : null}

      {tab === 'links' ? (
        links.length === 0 ? (
          <EmptyState compact icon="link" message={text('emptySharedBody', 'Shared media, files and pinned messages appear here.')} title={text('emptySharedTitle', 'No shared content')} />
        ) : (
          <div className="ac-details__block">
            <h3>{text('linksTab', 'Links')}</h3>
            {links.map(link => (
              <ListRow
                hint={link}
                icon="link"
                key={link}
                onClick={() => {
                  void navigator.clipboard
                    .writeText(link)
                    .then(() => toast(text('linkCopied', 'Link copied.')))
                    .catch(() => toast(text('featureSoon', 'This action is unavailable in the current build.'), 'neutral'));
                }}
                title={new URL(link).host}
                trailing={<Icon name="copy" size={15} />}
              />
            ))}
          </div>
        )
      ) : null}

      {tab === 'starred' ? (
        starred.length === 0 ? (
          <EmptyState compact icon="star" message={text('emptySharedBody', 'Shared media, files and pinned messages appear here.')} title={text('emptySharedTitle', 'No shared content')} />
        ) : (
          <div className="ac-details__block">
            <h3>{text('starredTab', 'Starred')}</h3>
            {starred.map(entry => (
              <ListRow
                hint={entry.text}
                icon="star"
                key={entry.id}
                title={nameOf(entry.sender)}
                trailing={<time>{messageTime(entry)}</time>}
              />
            ))}
          </div>
        )
      ) : null}

      {tab === 'pinned' ? (
        pinned.length === 0 ? (
          <EmptyState compact icon="pin" message={text('emptySharedBody', 'Shared media, files and pinned messages appear here.')} title={text('emptySharedTitle', 'No shared content')} />
        ) : (
          <div className="ac-details__block">
            <h3>{text('pinnedTab', 'Pinned')}</h3>
            {pinned.map(entry => (
              <ListRow
                hint={entry.text}
                icon="pin"
                key={entry.messageId}
                title={nameOf(entry.sender)}
                trailing={
                  <IconButton
                    iconSize={15}
                    label={text('unpin', 'Unpin')}
                    name="pinOff"
                    onClick={() => void bridge.pinMessage(chatId, entry.messageId, false).then(setPinned).catch((error: unknown) => toast(String(error), 'danger'))}
                  />
                }
              />
            ))}
          </div>
        )
      ) : null}

      {tab === 'members' && group ? (
        <div className="ac-details__block">
          <h3>
            {text('membersTab', 'Members')} · {members.length}
          </h3>
          {members.map(member => (
            <ListRow
              icon={member.online ? 'circle' : 'dot'}
              key={member.id}
              leading={
                <PeerAvatar
                  initials={initialsFor(member.name)}
                  name={member.name}
                  peerId={member.self ? null : member.id}
                  size={24}
                  source={bridge}
                />
              }
              onClick={member.self ? undefined : () => onOpenChat(member.id)}
              title={member.name}
              trailing={
                member.admin ? (
                  <Badge label={text('groupAdmin', 'Admin')} tone="accent">
                    {text('groupAdmin', 'Admin')}
                  </Badge>
                ) : null
              }
            />
          ))}
          {group.isAdmin && !group.isEveryone ? (
            <Button icon="usersRound" onClick={() => onOpenChat(chatId)} variant="secondary">
              {text('addMembers', 'Add members')}
            </Button>
          ) : null}
        </div>
      ) : null}

      <div className="ac-details__storage">
        <span>{text('storageUsed', '{{used}} used', { used: formatFileSize(usedBytes) })}</span>
        <span>{attachments.length} {text('filesTab', 'Files').toLowerCase()}</span>
      </div>

      <div className="ac-details__card">
        <h3 className="ac-details__block-title">{text('encryptionTitle', 'Encryption')}</h3>
        <p className="ac-composer__hint">
          <Icon name={settings.lanEncryption ? 'shieldCheck' : 'shield'} size={13} />
          {settings.lanEncryption
            ? text('encryptionOn', 'End-to-end encrypted on this local network')
            : text('encryptionOff', 'Not encrypted — set the network passphrase')}
        </p>
        <Tooltip content={text('copyFingerprint', 'Copy my fingerprint')} delay={200}>
          <div className="ac-details__key">{identity?.fingerprint ?? text('featureSoon', 'This action is unavailable in the current build.')}</div>
        </Tooltip>
      </div>

      <Button
        icon="download"
        onClick={() => {
          void bridge
            .exportData()
            .then(result => toast(text('exportReady', 'Export written to {{path}}', { path: result.path })))
            .catch(error => toast(String(error), 'danger'));
        }}
        variant="secondary"
      >
        {text('exportChat', 'Export this chat')}
      </Button>
      <Button icon="rotateCcw" onClick={() => setConfirm('clear')} variant="secondary">
        {text('clearChat', 'Clear chat')}
      </Button>
      {group && !group.isEveryone ? (
        <Button icon="logOut" onClick={() => setConfirm('leave')} variant="danger">
          {text('leaveGroup', 'Leave group')}
        </Button>
      ) : null}
      <Button icon="trash" onClick={() => setConfirm('delete')} variant="danger">
        {text('deleteChat', 'Delete chat')}
      </Button>

      {confirm ? (
        <div aria-label={text('detailsTitle', 'Chat details')} aria-modal="true" className="ac-chat-confirm" role="alertdialog">
          <div className="ac-chat-confirm__card">
            <span className="ac-chat-confirm__icon">
              <Icon name="alertCircle" size={20} />
            </span>
            <h3>
              {confirm === 'clear'
                ? text('clearChat', 'Clear chat')
                : confirm === 'leave'
                  ? text('leaveGroup', 'Leave group')
                  : text('deleteChat', 'Delete chat')}
            </h3>
            <p>
              {confirm === 'clear'
                ? text('confirmClear', 'Delete every message in this chat on this device?')
                : confirm === 'leave'
                  ? text('confirmLeaveGroup', 'You will stop receiving this group until you are added again.')
                  : text('confirmDeleteChat', 'Remove this chat and its history from this device?')}
            </p>
            <div className="ac-chat-confirm__actions">
              <button className="ac-btn ac-btn--secondary" onClick={() => setConfirm(null)} type="button">
                {text('cancel', 'Cancel')}
              </button>
              <button
                className="ac-btn ac-btn--danger"
                onClick={() => {
                  const kind = confirm;
                  setConfirm(null);
                  const call =
                    kind === 'clear'
                      ? bridge.clearChat(chatId).then(reload)
                      : kind === 'leave'
                        ? bridge.leaveGroup(chatId)
                        : bridge.deleteChat(chatId);
                  void call
                    .then(() => {
                      if (kind === 'delete') onClose();
                          toast(
                        kind === 'clear'
                          ? text('chatCleared', 'Chat history cleared on this device.')
                          : kind === 'leave'
                            ? text('leftGroup', 'You left the group.')
                            : text('chatDeleted', 'Chat deleted from this device.'),
                      );
                    })
                    .catch(error => toast(String(error), 'danger'));
                }}
                type="button"
              >
                {text('confirm', 'Confirm')}
              </button>
            </div>
          </div>
        </div>
      ) : null}
    </aside>
  );
}
