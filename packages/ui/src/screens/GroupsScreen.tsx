import { useCallback, useEffect, useMemo, useState, type MouseEvent, type ReactNode } from 'react';
import type { Group, Peer, PlatformBridge } from '../../../bridge/src';
import {
  AVATAR_PALETTE,
  Avatar,
  Badge,
  Banner,
  Button,
  ContextMenu,
  Dialog,
  EmptyState,
  Icon,
  IconButton,
  Input,
  ListGroup,
  ListRow,
  SearchInput,
  Segmented,
  Skeleton,
  Tooltip,
  avatarPaletteFor,
  contextMenuItems,
  cx,
  fileKindFor,
  initialsFor,
  type AvatarMember,
  type AvatarPaletteColor,
  type ContextMenuEntry,
  type IconName,
  type PresenceState,
} from '../components';
import { ns, registerNamespace } from '../i18n';
import groupsCatalog from '../i18n/en/groups';
import { ScreenHeader, type ScreenProps, type ToastTone } from './shared';
import {
  AVATAR_PREVIEW_SIZES,
  decodeIconSpec,
  droppedImage,
  encodeIconSpec,
  pickImageFile,
  prepareAvatarDataUrl,
  type AvatarFileRejection,
  type IconSpec,
} from './photoPipeline';
import './GroupsScreen.css';

registerNamespace('groups', groupsCatalog);
const text = ns('groups');

/* ========================================================================== *
 * Types + pure helpers
 * ========================================================================== */

const GROUP_NAME_MAX = 50;

const GROUP_EMOJIS = ['💼', '🎨', '🚀', '💰', '📚', '⭐', '🔥', '🎯', '🏠', '🎧', '😄', '🌙'];

const PRESENCE_RANK: Record<PresenceState, number> = { online: 0, away: 1, busy: 2, offline: 3 };

export type MemberView = {
  id: string;
  name: string;
  initials: string;
  color: AvatarPaletteColor;
  presence: PresenceState;
  status: string;
  isMe: boolean;
};

export type MediaEntry = {
  id: string;
  name: string;
  sender: string;
  time: string;
  glyph: IconName;
};

type MenuState = { x: number; y: number; label: string; entries: ContextMenuEntry[] };

function failure(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function rejectionMessage(reason: AvatarFileRejection): string {
  if (reason === 'type') return text('errorType', 'Only PNG, JPEG or WebP pictures can be used.');
  if (reason === 'size') return text('errorSize', 'That picture is bigger than 10 MB.');
  return text('errorEmpty', 'That file is empty.');
}

function paletteColor(value: string | undefined, seed: string): AvatarPaletteColor {
  return value && (AVATAR_PALETTE as readonly string[]).includes(value)
    ? (value as AvatarPaletteColor)
    : avatarPaletteFor(seed);
}

/** Presence for the dot and for the composite-avatar ordering rule. */
function presenceOf(peer: Peer): PresenceState {
  if (!peer.online) return 'offline';
  const status = peer.status.toLowerCase();
  if (status.includes('meeting') || status.includes('busy') || status.includes('dnd')) return 'busy';
  if (status.includes('away')) return 'away';
  return 'online';
}

function isSelf(id: string, peerId?: string | null): boolean {
  return id === 'me' || id === 'self' || (Boolean(peerId) && id === peerId);
}

const FILE_GLYPH_ICON: Record<string, IconName> = {
  media: 'image',
  doc: 'fileText',
  pdf: 'fileText',
  sheet: 'fileText',
  audio: 'fileAudio',
  archive: 'archive',
  file: 'file',
};

/**
 * Group rosters hold peer ids; `props.peers` carries the people. A peer the
 * network has not announced yet still renders, from its id alone.
 */
export function membersOf(
  group: Group,
  peers: readonly Peer[],
  displayName: string,
  avatarColor: string,
  peerId?: string | null,
): MemberView[] {
  const people = peers.filter((peer) => !peer.group);
  return group.members.map((id) => {
    const me = isSelf(id, peerId);
    const peer = people.find((person) => person.id === id);
    const name = me ? displayName.trim() || text('you', 'You') : peer?.name ?? id;
    return {
      color: me ? paletteColor(avatarColor, name) : paletteColor(peer?.color, name),
      id,
      initials: peer?.initials ?? initialsFor(name),
      isMe: me,
      name,
      presence: me ? 'online' : peer ? presenceOf(peer) : 'offline',
      status: me ? text('statusOnline', 'Online') : peer?.status ?? text('statusUnknown', 'Not seen on this network yet'),
    };
  });
}

function toAvatarMember(member: MemberView): AvatarMember {
  return { color: member.color, name: member.name, presence: member.presence };
}

/* ========================================================================== *
 * Group icon: photo > emoji + colour > Everyone glyph > composite members
 * ========================================================================== */

export function GroupIcon({
  group,
  members = [],
  size = 56,
  spec,
  cameraFallback = false,
}: {
  group?: Pick<Group, 'name' | 'icon' | 'isEveryone'> | null;
  members?: readonly MemberView[];
  size?: 24 | 40 | 56;
  spec?: IconSpec | null;
  cameraFallback?: boolean;
}) {
  const icon = spec === undefined ? decodeIconSpec(group?.icon) : spec;
  const name = group?.name ?? '';

  if (icon?.kind === 'photo') return <Avatar label={name} name={name} size={size} src={icon.src} />;
  if (icon?.kind === 'emoji') {
    return (
      <Avatar
        className={cx('grp-icon--emoji', `grp-icon--${size}`)}
        color={icon.color}
        initials={icon.emoji}
        label={name}
        name={name}
        size={size}
      />
    );
  }
  if (cameraFallback) return <Avatar group glyph="camera" label={name || text('noIcon', 'No icon chosen')} name={name} size={size} />;
  if (group?.isEveryone) return <Avatar group glyph="users" label={name} name={name} size={size} />;
  return <Avatar label={name} members={members.map(toAvatarMember)} name={name} size={size} />;
}

/* ========================================================================== *
 * Icon chooser — upload a picture OR emoji + palette colour, with live previews
 * ========================================================================== */

export function IconPicker({
  spec,
  onChange,
  toast,
}: {
  spec: IconSpec | null;
  onChange: (next: IconSpec | null) => void;
  toast: (message: string, tone?: ToastTone) => void;
}) {
  const [mode, setMode] = useState<'emoji' | 'photo'>(spec?.kind === 'photo' ? 'photo' : 'emoji');
  const [busy, setBusy] = useState(false);
  const [dropping, setDropping] = useState(false);

  const color: AvatarPaletteColor = spec?.kind === 'emoji' ? spec.color : 'violet';
  const emoji = spec?.kind === 'emoji' ? spec.emoji : GROUP_EMOJIS[0];

  const takeFile = useCallback(
    async (file: File | null) => {
      if (!file) return;
      setBusy(true);
      const result = await prepareAvatarDataUrl(file);
      setBusy(false);
      if (!result.ok) {
        toast(rejectionMessage(result.reason), 'danger');
        return;
      }
      setMode('photo');
      onChange({ kind: 'photo', src: result.dataUrl });
    },
    [onChange, toast],
  );

  return (
    <div className="grp-picker">
      <Segmented
        label={text('iconModeLabel', 'Group icon source')}
        onChange={(next) => setMode(next === 'photo' ? 'photo' : 'emoji')}
        options={[
          { icon: 'smile', label: text('iconModeEmoji', 'Emoji + colour'), value: 'emoji' },
          { icon: 'imagePlus', label: text('iconModePhoto', 'Upload photo'), value: 'photo' },
        ]}
        value={mode}
      />

      {mode === 'emoji' ? (
        <div className="grp-picker__emoji">
          <div aria-label={text('emojiLabel', 'Emoji')} className="grp-emoji-row" role="group">
            {GROUP_EMOJIS.map((option) => (
              <Button
                aria-label={text('emojiPick', 'Use {{emoji}} as the group icon', { emoji: option })}
                aria-pressed={option === emoji}
                className={cx('grp-emoji', option === emoji && 'grp-emoji--on')}
                key={option}
                onClick={() => onChange({ color, emoji: option, kind: 'emoji' })}
                size="sm"
                variant={option === emoji ? 'primary' : 'secondary'}
              >
                {option}
              </Button>
            ))}
          </div>
          <p className="grp-picker__note">{text('paletteHint', 'or choose a colour + emoji')}</p>
          <div aria-label={text('paletteLabel', 'Icon colour')} className="grp-palette" role="group">
            {AVATAR_PALETTE.map((option) => (
              <Button
                aria-label={text('paletteColor', '{{color}} icon colour', { color: option })}
                aria-pressed={option === color}
                className={cx('grp-swatch', `grp-swatch--${option}`, option === color && 'grp-swatch--on')}
                key={option}
                onClick={() => onChange({ color: option, emoji, kind: 'emoji' })}
                size="sm"
                variant="ghost"
              />
            ))}
          </div>
        </div>
      ) : (
        <div className="grp-picker__photo">
          <div
            aria-label={text('dropHint', 'Drag a picture here')}
            className={cx('grp-drop', dropping && 'grp-drop--on')}
            onDragOver={(event) => {
              event.preventDefault();
              setDropping(true);
            }}
            onDragLeave={() => setDropping(false)}
            onDrop={(event) => {
              event.preventDefault();
              setDropping(false);
              void takeFile(droppedImage(event));
            }}
            role="presentation"
          >
            <Icon name="cloudUpload" size={22} />
            <span>{text('dropHint', 'Drag a picture here')}</span>
          </div>
          <div className="grp-picker__photo-actions">
            <Button icon="upload" loading={busy} onClick={() => void pickImageFile().then(takeFile)} variant="secondary">
              {text('chooseFile', 'Choose photo…')}
            </Button>
            <Button disabled={spec?.kind !== 'photo'} icon="trash" onClick={() => onChange(null)} variant="ghost">
              {text('clearIcon', 'Remove')}
            </Button>
          </div>
          <p className="grp-picker__note">{text('uploadHint', 'Pictures are resized to 256 px and stripped of EXIF before they are stored.')}</p>
        </div>
      )}

      <div className="grp-previews">
        <span className="grp-previews__label">{text('previewLabel', 'Preview')}</span>
        {AVATAR_PREVIEW_SIZES.map((size) => (
          <span className="grp-preview" key={size}>
            <GroupIcon cameraFallback size={size} spec={spec} />
            <span className="grp-preview__px">{text('previewPx', '{{size}} px', { size })}</span>
          </span>
        ))}
      </div>
    </div>
  );
}

/* ========================================================================== *
 * Member picker (shared by New Group and Add Members)
 * ========================================================================== */

export function MemberPicker({
  peers,
  selected,
  onToggle,
  onSelectAll,
  onClear,
  excludeIds = [],
}: {
  peers: readonly Peer[];
  selected: readonly string[];
  onToggle: (id: string) => void;
  onSelectAll: (ids: readonly string[]) => void;
  onClear: () => void;
  excludeIds?: readonly string[];
}) {
  const [query, setQuery] = useState('');
  const people = useMemo(
    () => peers.filter((peer) => !peer.group && !excludeIds.includes(peer.id)),
    [excludeIds, peers],
  );
  const needle = query.trim().toLowerCase();
  const matches = useMemo(
    () => people.filter((peer) => needle.length === 0 || peer.name.toLowerCase().includes(needle)),
    [needle, people],
  );
  const online = matches.filter((peer) => peer.online);
  const offline = matches.filter((peer) => !peer.online);
  const selectedPeers = people.filter((peer) => selected.includes(peer.id));

  return (
    <div className="grp-members">
      <SearchInput
        aria-label={text('searchContacts', 'Search contacts…')}
        onChange={setQuery}
        onClear={() => setQuery('')}
        placeholder={text('searchContacts', 'Search contacts…')}
        value={query}
      />

      {selectedPeers.length > 0 ? (
        <div className="grp-chips-block">
          <div className="grp-block-head">
            <span className="grp-field-label">{text('selectedCount', 'Selected ({{count}})', { count: selectedPeers.length })}</span>
            <Button onClick={onClear} size="sm" variant="ghost">
              {text('clearSelection', 'Clear')}
            </Button>
          </div>
          <div className="grp-chips">
            {selectedPeers.map((peer) => (
              <span className="grp-chip" key={peer.id}>
                <Avatar
                  color={paletteColor(peer.color, peer.name)}
                  name={peer.name}
                  presence={presenceOf(peer)}
                  size={24}
                />
                <span className="grp-chip__name">{peer.name}</span>
                <IconButton
                  iconSize={12}
                  label={text('removeMember', 'Remove {{name}} from the selection', { name: peer.name })}
                  name="x"
                  onClick={() => onToggle(peer.id)}
                  size="sm"
                />
              </span>
            ))}
          </div>
        </div>
      ) : null}

      <div className="grp-block-head">
        <span className="grp-field-label">{text('onlineCount', 'Online ({{count}})', { count: online.length })}</span>
        <Button
          disabled={online.length === 0}
          onClick={() => onSelectAll(online.map((peer) => peer.id))}
          size="sm"
          variant="ghost"
        >
          {text('selectAll', 'Select All')}
        </Button>
      </div>

      <div className="grp-picker__list">
        {matches.length === 0 ? (
          <EmptyState
            compact
            icon="searchX"
            message={text('noPeopleHint', 'People appear here once AlverChat finds them on the local network.')}
            title={needle.length > 0 ? text('noPeopleQuery', 'Nobody matches “{{query}}”', { query: needle }) : text('noPeople', 'No people found yet')}
          />
        ) : (
          <ListGroup>
            {online.map((peer) => (
              <MemberCheckRow key={peer.id} onToggle={onToggle} peer={peer} selected={selected.includes(peer.id)} />
            ))}
            {offline.length > 0 ? (
              <>
                <div className="grp-section-label">{text('offlineCount', 'Offline ({{count}})', { count: offline.length })}</div>
                {offline.map((peer) => (
                  <MemberCheckRow key={peer.id} onToggle={onToggle} peer={peer} selected={selected.includes(peer.id)} />
                ))}
              </>
            ) : null}
          </ListGroup>
        )}
      </div>
    </div>
  );
}

function MemberCheckRow({ peer, selected, onToggle }: { peer: Peer; selected: boolean; onToggle: (id: string) => void }) {
  return (
    <ListRow
      aria-pressed={selected}
      className="grp-check-row"
      hint={peer.status}
      leading={
        <>
          <span aria-hidden className={cx('grp-check', selected && 'grp-check--on')}>
            {selected ? <Icon name="check" size={13} /> : null}
          </span>
          <Avatar color={paletteColor(peer.color, peer.name)} name={peer.name} presence={presenceOf(peer)} size={40} />
        </>
      }
      onClick={() => onToggle(peer.id)}
      title={peer.name}
    />
  );
}

/* ========================================================================== *
 * Dialogs
 * ========================================================================== */

export function NewGroupDialog({
  open,
  onOpenChange,
  peers,
  bridge,
  toast,
  onCreated,
}: {
  open: boolean;
  onOpenChange: (next: boolean) => void;
  peers: readonly Peer[];
  bridge: PlatformBridge;
  toast: (message: string, tone?: ToastTone) => void;
  onCreated: (group: Group) => void;
}) {
  const [name, setName] = useState('');
  const [icon, setIcon] = useState<IconSpec | null>(null);
  const [selected, setSelected] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);

  const reset = useCallback(() => {
    setName('');
    setIcon(null);
    setSelected([]);
    setBusy(false);
  }, []);

  const trimmed = name.trim();
  const error =
    trimmed.length === 0
      ? text('nameRequired', 'Give the group a name.')
      : trimmed.length > GROUP_NAME_MAX
        ? text('nameTooLong', 'Group names are limited to {{max}} characters.', { max: GROUP_NAME_MAX })
        : '';

  const toggle = (id: string) =>
    setSelected((current) => (current.includes(id) ? current.filter((item) => item !== id) : [...current, id]));

  async function create() {
    if (error) return;
    setBusy(true);
    try {
      const group = await bridge.createGroup(trimmed, selected, encodeIconSpec(icon));
      onCreated(group);
      toast(text('createdToast', '“{{name}}” created', { name: group.name }), 'success');
      reset();
      onOpenChange(false);
    } catch (cause) {
      toast(failure(cause), 'danger');
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog
      description={text('newGroupDescription', 'Pick an icon, a name and the people who should be in this group.')}
      footer={
        <>
          <Button onClick={() => onOpenChange(false)} variant="secondary">
            {text('cancel', 'Cancel')}
          </Button>
          <Button disabled={Boolean(error)} icon="check" loading={busy} onClick={() => void create()}>
            {text('create', 'Create')}
          </Button>
        </>
      }
      onOpenChange={(next) => {
        if (!next) reset();
        onOpenChange(next);
      }}
      open={open}
      size="lg"
      title={text('newGroupTitle', 'New Group')}
    >
      <div className="grp-dialog-grid">
        <div className="grp-dialog__preview">
          <GroupIcon cameraFallback size={56} spec={icon} />
          <span className="grp-dialog__preview-name">{trimmed || text('groupNamePlaceholder', 'Group name')}</span>
        </div>
        <IconPicker onChange={setIcon} spec={icon} toast={toast} />
      </div>

      <Input
        error={name.length > 0 && error ? error : undefined}
        helper={text('nameHelper', 'Up to {{max}} characters.', { max: GROUP_NAME_MAX })}
        label={text('nameLabel', 'Group name')}
        maxLength={GROUP_NAME_MAX}
        onChange={(event) => setName(event.target.value)}
        placeholder={text('namePlaceholder', 'Design Team')}
        showCount
        value={name}
      />

      <div className="grp-dialog__members">
        <span className="grp-field-label">{text('addMembers', 'Add members')}</span>
        <MemberPicker
          onClear={() => setSelected([])}
          onSelectAll={(ids) => setSelected([...ids])}
          onToggle={toggle}
          peers={peers}
          selected={selected}
        />
      </div>
    </Dialog>
  );
}

export function ChangeIconDialog({
  group,
  onOpenChange,
  bridge,
  toast,
  onUpdated,
}: {
  group: Group | null;
  onOpenChange: (next: boolean) => void;
  bridge: PlatformBridge;
  toast: (message: string, tone?: ToastTone) => void;
  onUpdated: (group: Group) => void;
}) {
  const [spec, setSpec] = useState<IconSpec | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (group) setSpec(decodeIconSpec(group.icon));
  }, [group]);

  async function save() {
    if (!group) return;
    setBusy(true);
    try {
      const next = await bridge.updateGroup(group.id, { icon: encodeIconSpec(spec) });
      onUpdated(next);
      toast(text('iconSavedToast', 'Group icon updated'), 'success');
      onOpenChange(false);
    } catch (cause) {
      toast(failure(cause), 'danger');
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog
      description={text('changeIconDescription', 'Pick a new icon for {{name}}. Every member sees it.', { name: group?.name ?? '' })}
      footer={
        <>
          <Button onClick={() => onOpenChange(false)} variant="secondary">
            {text('cancel', 'Cancel')}
          </Button>
          <Button icon="check" loading={busy} onClick={() => void save()}>
            {text('save', 'Save')}
          </Button>
        </>
      }
      onOpenChange={onOpenChange}
      open={Boolean(group)}
      size="lg"
      title={text('changeIconTitle', 'Change Group Icon')}
    >
      <div className="grp-dialog-grid">
        <div className="grp-dialog__preview">
          <GroupIcon cameraFallback group={group} size={56} spec={spec} />
          <span className="grp-dialog__preview-name">{group?.name}</span>
        </div>
        <IconPicker onChange={setSpec} spec={spec} toast={toast} />
      </div>
    </Dialog>
  );
}

function AddMembersDialog({
  group,
  peers,
  onOpenChange,
  bridge,
  toast,
  onUpdated,
}: {
  group: Group | null;
  peers: readonly Peer[];
  onOpenChange: (next: boolean) => void;
  bridge: PlatformBridge;
  toast: (message: string, tone?: ToastTone) => void;
  onUpdated: (group: Group) => void;
}) {
  const [selected, setSelected] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (group) setSelected([]);
  }, [group]);

  async function add() {
    if (!group || selected.length === 0) return;
    setBusy(true);
    try {
      const next = await bridge.addGroupMembers(group.id, selected);
      onUpdated(next);
      toast(text('membersAddedToast', '{{count}} members added', { count: selected.length }), 'success');
      onOpenChange(false);
    } catch (cause) {
      toast(failure(cause), 'danger');
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog
      description={text('addMembersDescription', 'Choose who to add to {{name}}.', { name: group?.name ?? '' })}
      footer={
        <>
          <Button onClick={() => onOpenChange(false)} variant="secondary">
            {text('cancel', 'Cancel')}
          </Button>
          <Button disabled={selected.length === 0} icon="plus" loading={busy} onClick={() => void add()}>
            {text('addMembers', 'Add members')}
          </Button>
        </>
      }
      onOpenChange={onOpenChange}
      open={Boolean(group)}
      size="lg"
      title={text('addMembersTitle', 'Add members')}
    >
      <MemberPicker
        excludeIds={group?.members ?? []}
        onClear={() => setSelected([])}
        onSelectAll={(ids) => setSelected([...ids])}
        onToggle={(id) =>
          setSelected((current) => (current.includes(id) ? current.filter((item) => item !== id) : [...current, id]))
        }
        peers={peers}
        selected={selected}
      />
    </Dialog>
  );
}

function FieldDialog({
  open,
  onOpenChange,
  title,
  description,
  label,
  value,
  onChange,
  onSubmit,
  submitLabel,
  maxLength,
  helper,
  busy,
}: {
  open: boolean;
  onOpenChange: (next: boolean) => void;
  title: string;
  description?: string;
  label: string;
  value: string;
  onChange: (next: string) => void;
  onSubmit: () => void;
  submitLabel: string;
  maxLength?: number;
  helper?: string;
  busy: boolean;
}) {
  return (
    <Dialog
      description={description}
      footer={
        <>
          <Button onClick={() => onOpenChange(false)} variant="secondary">
            {text('cancel', 'Cancel')}
          </Button>
          <Button disabled={value.trim().length === 0} icon="check" loading={busy} onClick={onSubmit}>
            {submitLabel}
          </Button>
        </>
      }
      onOpenChange={onOpenChange}
      open={open}
      title={title}
    >
      <Input
        helper={helper}
        label={label}
        maxLength={maxLength}
        onChange={(event) => onChange(event.target.value)}
        showCount={Boolean(maxLength)}
        value={value}
      />
    </Dialog>
  );
}

/* ========================================================================== *
 * Action tile + group info panel
 * ========================================================================== */

function ActionTile({
  icon,
  label,
  onClick,
  disabled = false,
  active = false,
  blockedHint,
}: {
  icon: IconName;
  label: string;
  onClick: (event: MouseEvent<HTMLElement>) => void;
  disabled?: boolean;
  active?: boolean;
  blockedHint?: string;
}) {
  const button = (
    <IconButton
      active={active}
      disabled={disabled}
      label={label}
      name={icon}
      onClick={onClick}
      size="lg"
      variant="soft"
    />
  );
  return (
    <span className="grp-action">
      {disabled && blockedHint ? (
        <Tooltip content={blockedHint} wrap>
          {button}
        </Tooltip>
      ) : (
        button
      )}
      <span className="grp-action__label">{label}</span>
    </span>
  );
}

export function GroupInfoPanel({
  group,
  members,
  media,
  muted,
  announcementsFromAdminsOnly,
  onClose,
  onOpenChat,
  onMute,
  onRequestAdd,
  onRequestRename,
  onRequestIcon,
  onRequestDescription,
  onRequestLeave,
  onRemoveMember,
  onMessageMember,
  onMakeAdmin,
  onOpenMenu,
}: {
  group: Group;
  members: readonly MemberView[];
  media: readonly MediaEntry[];
  muted: boolean;
  announcementsFromAdminsOnly: boolean;
  onClose: () => void;
  onOpenChat: () => void;
  onMute: (next: boolean) => void;
  onRequestAdd: () => void;
  onRequestRename: () => void;
  onRequestIcon: () => void;
  onRequestDescription: () => void;
  onRequestLeave: () => void;
  onRemoveMember: (member: MemberView) => void;
  onMessageMember: (member: MemberView) => void;
  onMakeAdmin: (member: MemberView) => void;
  onOpenMenu: (event: MouseEvent<HTMLElement>, label: string, entries: ContextMenuEntry[]) => void;
}) {
  const [query, setQuery] = useState('');
  const [searchOpen, setSearchOpen] = useState(false);
  const [sort, setSort] = useState<'presence' | 'name'>('presence');
  const [openSection, setOpenSection] = useState<'media' | 'pinned' | null>(null);

  const isEveryone = Boolean(group.isEveryone);
  const canManage = Boolean(group.isAdmin) && !isEveryone;
  const needle = query.trim().toLowerCase();

  useEffect(() => {
    setOpenSection(null);
    setSearchOpen(false);
    setQuery('');
  }, [group.id]);

  const shownMembers = useMemo(() => {
    const filtered = members.filter((member) => needle.length === 0 || member.name.toLowerCase().includes(needle));
    return [...filtered].sort((a, b) => {
      if (sort === 'name') return a.name.localeCompare(b.name);
      const rank = PRESENCE_RANK[a.presence] - PRESENCE_RANK[b.presence];
      return rank === 0 ? a.name.localeCompare(b.name) : rank;
    });
  }, [members, needle, sort]);

  const shownMedia = media.filter((item) => needle.length === 0 || item.name.toLowerCase().includes(needle));

  const moreEntries: ContextMenuEntry[] = [
    ...contextMenuItems([
      { disabled: !canManage, icon: 'pencil', id: 'rename', label: text('renameGroup', 'Rename group'), onSelect: onRequestRename },
      { disabled: !canManage, icon: 'imagePlus', id: 'icon', label: text('changeIcon', 'Change icon'), onSelect: onRequestIcon },
      {
        disabled: !canManage,
        icon: 'fileText',
        id: 'description',
        label: text('editDescription', 'Edit description'),
        onSelect: onRequestDescription,
      },
    ]),
    { id: 'group-sep', kind: 'separator' },
    ...contextMenuItems([
      {
        danger: true,
        disabled: isEveryone,
        icon: 'logOut',
        id: 'leave',
        label: text('leaveGroup', 'Leave Group'),
        onSelect: onRequestLeave,
      },
    ]),
  ];

  const sortEntries = contextMenuItems([
    {
      checked: sort === 'presence',
      icon: 'users',
      id: 'presence',
      label: text('sortPresence', 'Online first'),
      onSelect: () => setSort('presence'),
    },
    {
      checked: sort === 'name',
      icon: 'list',
      id: 'name',
      label: text('sortName', 'Name (A–Z)'),
      onSelect: () => setSort('name'),
    },
  ]);

  return (
    <aside aria-label={text('panelLabel', 'Info for {{name}}', { name: group.name })} className="grp-panel">
      <header className="grp-panel__head">
        <span className="grp-panel__eyebrow">{text('groupInfo', 'Group Info')}</span>
        <IconButton iconSize={15} label={text('closePanel', 'Close group info')} name="x" onClick={onClose} size="sm" />
      </header>

      <div className="grp-panel__identity">
        <GroupIcon group={group} members={members} size={56} />
        <h3 className="grp-panel__name">{group.name}</h3>
        <p className="grp-panel__count">
          <span
            aria-hidden
            className={cx('grp-dot', members.some((member) => member.presence === 'online') ? 'grp-dot--on' : 'grp-dot--off')}
          />
          {isEveryone ? text('allMembersCount', 'All members ({{count}})', { count: members.length }) : text('membersCount', '{{count}} members', { count: members.length })}
        </p>
        <p className="grp-panel__description">{group.description?.trim() || text('noDescription', 'No description yet.')}</p>
      </div>

      {isEveryone ? (
        <Banner icon="globe" message={text('everyoneBannerMessage', 'This group is every peer AlverChat can currently see on your local network. The roster, the icon and the admin role are managed by the network, not by you.')} title={text('everyoneBannerTitle', 'Everyone follows the network')} tone="info" />
      ) : null}

      <p className="grp-panel__announce">
        <Icon name="megaphone" size={15} />
        {announcementsFromAdminsOnly ? text('announcementsAdmins', 'Only group admins can post announcements here.') : text('announcementsAll', 'Anyone in the group can post announcements.')}
      </p>

      <div className="grp-actions">
        <ActionTile icon="messageCircle" label={text('actionChat', 'Chat')} onClick={onOpenChat} />
        {isEveryone ? (
          <>
            <ActionTile
              icon="folder"
              label={text('actionFiles', 'Files')}
              onClick={() => setOpenSection(openSection === 'media' ? null : 'media')}
            />
            <ActionTile
              icon="pin"
              label={text('actionPinned', 'Pinned')}
              onClick={() => setOpenSection(openSection === 'pinned' ? null : 'pinned')}
            />
          </>
        ) : (
          <>
            <ActionTile
              blockedHint={text('adminsOnly', 'Group admins only.')}
              disabled={!canManage}
              icon="plus"
              label={text('actionAdd', 'Add')}
              onClick={onRequestAdd}
            />
            <ActionTile
              active={muted}
              icon={muted ? 'bellOff' : 'bell'}
              label={muted ? text('actionUnmute', 'Unmute') : text('actionMute', 'Mute')}
              onClick={() => onMute(!muted)}
            />
            <ActionTile
              active={searchOpen}
              icon="search"
              label={text('actionSearch', 'Search')}
              onClick={() => {
                setSearchOpen((current) => !current);
                setQuery('');
              }}
            />
          </>
        )}
        <ActionTile
          icon="moreHorizontal"
          label={text('actionMore', 'More')}
          onClick={(event) => onOpenMenu(event, text('moreMenuLabel', 'More group actions'), moreEntries)}
        />
      </div>

      {searchOpen ? (
        <SearchInput
          aria-label={text('searchConversation', 'Search this conversation')}
          onChange={setQuery}
          onClear={() => setQuery('')}
          placeholder={text('searchConversation', 'Search this conversation')}
          value={query}
        />
      ) : null}

      <div className="grp-panel__section-head">
        <span className="grp-field-label">{text('membersSection', 'Members ({{count}})', { count: members.length })}</span>
        <Button
          icon="list"
          iconSide="right"
          onClick={(event) => onOpenMenu(event, text('sortMenuLabel', 'Sort members'), sortEntries)}
          size="sm"
          variant="ghost"
        >
          {text('sort', 'Sort')}
        </Button>
      </div>

      <ListGroup className="grp-members-list">
        {shownMembers.length === 0 ? (
          <EmptyState compact icon="users" message={text('noMemberMatches', 'Nobody here matches your search yet.')} title={text('noMembersFound', 'No members match the search')} />
        ) : (
          shownMembers.map((member) => {
            const memberEntries: ContextMenuEntry[] = contextMenuItems([
              {
                disabled: member.isMe,
                icon: 'messageCircle',
                id: 'message',
                label: text('memberMessage', 'Message'),
                onSelect: () => onMessageMember(member),
              },
              {
                disabled: !canManage,
                icon: 'crown',
                id: 'make-admin',
                label: text('memberMakeAdmin', 'Make admin'),
                onSelect: () => onMakeAdmin(member),
              },
              {
                danger: true,
                disabled: !canManage || member.isMe,
                icon: 'trash',
                id: 'remove',
                label: text('memberRemove', 'Remove'),
                onSelect: () => onRemoveMember(member),
              },
            ]);
            const row = (
              <ListRow
                className="grp-member-row"
                hint={member.status}
                key={member.id}
                leading={
                  <Avatar color={member.color} initials={member.initials} name={member.name} presence={member.presence} size={40} />
                }
                title={
                  <span className="grp-member-title">
                    <span className="grp-member-name">{member.isMe ? text('memberYou', '{{name}} (You)', { name: member.name }) : member.name}</span>
                    {member.id === group.admin ? <Badge tone="accent">{text('adminTag', 'Admin')}</Badge> : null}
                  </span>
                }
                trailing={
                  <IconButton
                    iconSize={16}
                    label={text('memberMenuLabel', 'Actions for {{name}}', { name: member.name })}
                    name="moreHorizontal"
                    onClick={(event) => onOpenMenu(event, text('memberMenuLabel', 'Actions for {{name}}', { name: member.name }), memberEntries)}
                    size="sm"
                  />
                }
              />
            );
            return canManage ? (
              row
            ) : (
              <Tooltip content={text('memberMenuLockedHint', 'Only the group admin can change members or roles.')} key={member.id} wrap>
                {row}
              </Tooltip>
            );
          })
        )}
      </ListGroup>

      <div className="grp-panel__sections">
        <ListRow
          chevron
          icon="image"
          onClick={() => setOpenSection(openSection === 'media' ? null : 'media')}
          title={text('sharedMedia', 'Shared Media')}
          trailing={<span className="grp-count">{text('itemsCount', '{{count}} items', { count: media.length })}</span>}
        />
        {openSection === 'media' ? (
          shownMedia.length === 0 ? (
            <EmptyState compact icon="image" message={text('noMediaMessage', 'Photos and files sent in this group appear here.')} title={text('noMediaTitle', 'No shared media yet')} />
          ) : (
            <div className="grp-media-grid">
              {shownMedia.map((item) => (
                <span className="grp-media-tile" key={item.id}>
                  <Icon name={item.glyph} size={22} />
                  <span className="grp-media-tile__name">{item.name}</span>
                  <span className="grp-media-tile__meta">
                    {item.sender} · {item.time}
                  </span>
                </span>
              ))}
            </div>
          )
        ) : null}

        <ListRow
          chevron
          icon="pin"
          onClick={() => setOpenSection(openSection === 'pinned' ? null : 'pinned')}
          title={text('pinnedMessages', 'Pinned Messages')}
          trailing={<span className="grp-count">{text('itemsCount', '{{count}} items', { count: 0 })}</span>}
        />
        {openSection === 'pinned' ? (
          <EmptyState compact icon="pin" message={text('noPinnedMessage', 'Pin a message in the conversation to keep it within reach.')} title={text('noPinnedTitle', 'No pinned messages')} />
        ) : null}
      </div>

      {isEveryone ? (
        <Tooltip content={text('everyoneLeaveHint', 'The Everyone group is the network itself — there is nothing to leave.')} wrap>
          <span className="grp-leave-slot">
            <Button block disabled icon="logOut" variant="danger">
              {text('leaveGroup', 'Leave Group')}
            </Button>
          </span>
        </Tooltip>
      ) : (
        <Button block icon="logOut" onClick={onRequestLeave} variant="danger">
          {text('leaveGroup', 'Leave Group')}
        </Button>
      )}
    </aside>
  );
}

/* ========================================================================== *
 * Screen
 * ========================================================================== */

export function GroupsScreen(props: ScreenProps) {
  const { bridge, peers, settings, identity, toast, go } = props;
  const [groups, setGroups] = useState<readonly Group[] | null>(null);
  const [layout, setLayout] = useState<'grid' | 'list'>('grid');
  const [query, setQuery] = useState('');
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [newOpen, setNewOpen] = useState(false);
  const [iconTarget, setIconTarget] = useState<Group | null>(null);
  const [addTarget, setAddTarget] = useState<Group | null>(null);
  const [leaveTarget, setLeaveTarget] = useState<Group | null>(null);
  const [renameTarget, setRenameTarget] = useState<Group | null>(null);
  const [renameValue, setRenameValue] = useState('');
  const [descriptionTarget, setDescriptionTarget] = useState<Group | null>(null);
  const [descriptionValue, setDescriptionValue] = useState('');
  const [busy, setBusy] = useState(false);
  const [menu, setMenu] = useState<MenuState | null>(null);
  const [media, setMedia] = useState<readonly MediaEntry[]>([]);

  const refresh = useCallback(async () => {
    try {
      setGroups(await bridge.listGroups());
    } catch (cause) {
      toast(failure(cause), 'danger');
    }
  }, [bridge, toast]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const selected = useMemo(
    () => (groups ?? []).find((group) => group.id === selectedId) ?? null,
    [groups, selectedId],
  );

  useEffect(() => {
    let live = true;
    if (!selected) {
      setMedia([]);
      return;
    }
    void bridge
      .listMessages(selected.id)
      .then((messages) => {
        if (!live) return;
        setMedia(
          messages
            .filter((message) => message.kind === 'file')
            .map((message) => ({
              glyph: FILE_GLYPH_ICON[fileKindFor(message.fileName ?? message.text)] ?? 'file',
              id: message.id,
              name: message.fileName ?? message.text,
              sender: message.sender,
              time: message.time,
            })),
        );
      })
      .catch(() => {
        if (live) setMedia([]);
      });
    return () => {
      live = false;
    };
  }, [bridge, selected]);

  const applyGroup = useCallback((next: Group) => {
    setGroups((current) => {
      const list = current ?? [];
      return list.some((group) => group.id === next.id)
        ? list.map((group) => (group.id === next.id ? next : group))
        : [next, ...list];
    });
  }, []);

  const openMenu = useCallback((event: MouseEvent<HTMLElement>, label: string, entries: ContextMenuEntry[]) => {
    const box = event.currentTarget.getBoundingClientRect();
    setMenu({ entries, label, x: box.left, y: box.bottom + 6 });
  }, []);

  const membersFor = useCallback(
    (group: Group) => membersOf(group, peers, settings.displayName, settings.avatarColor, identity?.peerId),
    [identity?.peerId, peers, settings.avatarColor, settings.displayName],
  );

  const needle = query.trim().toLowerCase();
  const visibleGroups = useMemo(
    () => (groups ?? []).filter((group) => needle.length === 0 || group.name.toLowerCase().includes(needle)),
    [groups, needle],
  );

  const unreadFor = useCallback((group: Group) => peers.find((peer) => peer.id === group.id)?.unread ?? 0, [peers]);
  const mutedFor = useCallback(
    (group: Group) =>
      settings.mutedChats.includes(group.id) || Boolean(peers.find((peer) => peer.id === group.id)?.muted),
    [peers, settings.mutedChats],
  );

  async function toggleMute(group: Group, next: boolean) {
    try {
      await bridge.setMutedChat(group.id, next);
      toast(next ? text('mutedToast', '“{{name}}” muted on this device', { name: group.name }) : text('unmutedToast', '“{{name}}” is no longer muted', { name: group.name }), 'success');
    } catch (cause) {
      toast(failure(cause), 'danger');
    }
  }

  async function removeMember(group: Group, member: MemberView) {
    try {
      applyGroup(await bridge.removeGroupMember(group.id, member.id));
      toast(text('removedToast', '{{name}} was removed', { name: member.name }), 'success');
    } catch (cause) {
      toast(failure(cause), 'danger');
    }
  }

  async function confirmLeave() {
    if (!leaveTarget) return;
    const name = leaveTarget.name;
    setBusy(true);
    try {
      await bridge.leaveGroup(leaveTarget.id);
      setSelectedId(null);
      setLeaveTarget(null);
      await refresh();
      toast(text('leftToast', 'You left “{{name}}”', { name }), 'success');
    } catch (cause) {
      toast(failure(cause), 'danger');
    } finally {
      setBusy(false);
    }
  }

  async function submitRename() {
    if (!renameTarget) return;
    setBusy(true);
    try {
      applyGroup(await bridge.updateGroup(renameTarget.id, { name: renameValue.trim() }));
      setRenameTarget(null);
      toast(text('renamedToast', 'Group renamed'), 'success');
    } catch (cause) {
      toast(failure(cause), 'danger');
    } finally {
      setBusy(false);
    }
  }

  async function submitDescription() {
    if (!descriptionTarget) return;
    setBusy(true);
    try {
      applyGroup(await bridge.updateGroup(descriptionTarget.id, { description: descriptionValue.trim() }));
      setDescriptionTarget(null);
      toast(text('descriptionSavedToast', 'Description saved'), 'success');
    } catch (cause) {
      toast(failure(cause), 'danger');
    } finally {
      setBusy(false);
    }
  }

  let body: ReactNode;
  if (groups === null) {
    body = (
      <div className="grp-grid">
        {[0, 1, 2, 3].map((key) => (
          <span className="grp-card-skeleton" key={key}>
            <Skeleton variant="avatar" />
            <Skeleton variant="title" />
            <Skeleton variant="text" />
          </span>
        ))}
      </div>
    );
  } else if (visibleGroups.length === 0) {
    body =
      groups.length === 0 ? (
        <EmptyState
          icon="users"
          message={text('emptyMessage', 'Select peers on any device and start a group in one tap.')}
          title={text('emptyTitle', 'No groups yet')}
          actions={
            <Button icon="plus" onClick={() => setNewOpen(true)}>
              {text('newGroup', 'New group')}
            </Button>
          }
        />
      ) : (
        <EmptyState
          icon="searchX"
          message={text('noGroupsMatchHint', 'Try another name, or start a new group.')}
          title={text('noGroupsMatch', 'No groups match “{{query}}”', { query: needle })}
          actions={
            <Button variant="secondary" onClick={() => setQuery('')}>
              {text('clearSearch', 'Clear search')}
            </Button>
          }
        />
      );
  } else {
    body = (
      <div className={layout === 'grid' ? 'grp-grid' : 'grp-list'}>
        {visibleGroups.map((group) => {
          const members = membersFor(group);
          const unread = unreadFor(group);
          return (
            <ListRow
              aria-pressed={group.id === selectedId}
              className={cx(
                'grp-card',
                layout === 'grid' && 'grp-card--grid',
                group.id === selectedId && 'grp-card--active',
              )}
              current={group.id === selectedId}
              hint={
                group.isEveryone
                  ? text('allMembersCount', 'All members ({{count}})', { count: members.length })
                  : text('membersCount', '{{count}} members', { count: members.length })
              }
              key={group.id}
              leading={<GroupIcon group={group} members={members} size={layout === 'grid' ? 56 : 40} />}
              onClick={() => setSelectedId(group.id)}
              title={
                <span className="grp-card__title">
                  <span className="grp-card__name">{group.name}</span>
                  {group.isEveryone ? <Badge tone="outline">{text('everyoneTag', 'Network')}</Badge> : null}
                </span>
              }
              trailing={
                <>
                  {mutedFor(group) ? <Icon name="bellOff" size={15} /> : null}
                  {unread > 0 ? <Badge count={unread} tone="accent" /> : null}
                </>
              }
            />
          );
        })}
      </div>
    );
  }

  return (
    <div className="grp-screen">
      <ScreenHeader
        actions={
          <>
            <SearchInput
              aria-label={text('searchGroups', 'Search groups…')}
              onChange={setQuery}
              onClear={() => setQuery('')}
              placeholder={text('searchGroups', 'Search groups…')}
              value={query}
            />
            <Segmented
              label={text('layoutLabel', 'Layout')}
              onChange={(next) => setLayout(next === 'list' ? 'list' : 'grid')}
              options={[
                { icon: 'grid', label: text('layoutGrid', 'Grid'), value: 'grid' },
                { icon: 'list', label: text('layoutList', 'List'), value: 'list' },
              ]}
              value={layout}
            />
            <Button icon="plus" onClick={() => setNewOpen(true)}>
              {text('newGroup', 'New group')}
            </Button>
          </>
        }
        description={text('description', 'Group rosters are signed on this device and synced to every member over the local network.')}
        eyebrow={text('eyebrow', 'TEAM CONVERSATIONS')}
        title={text('title', 'Groups')}
      />

      <div className="grp-body">
        <section className="grp-main">{body}</section>
        {selected ? (
          <GroupInfoPanel
            announcementsFromAdminsOnly={settings.announcementsFromAdminsOnly}
            group={selected}
            media={media}
            members={membersFor(selected)}
            muted={mutedFor(selected)}
            onClose={() => setSelectedId(null)}
            onMakeAdmin={(member) =>
              toast(text('memberRolePending', 'Admin roles move with the next signed group epoch — not wired to the bridge yet.', { name: member.name }), 'neutral')
            }
            onMessageMember={(member) => go('chats', member.id)}
            onMute={(next) => void toggleMute(selected, next)}
            onOpenChat={() => go('chats', selected.id)}
            onOpenMenu={openMenu}
            onRemoveMember={(member) => void removeMember(selected, member)}
            onRequestAdd={() => setAddTarget(selected)}
            onRequestDescription={() => {
              setDescriptionValue(selected.description ?? '');
              setDescriptionTarget(selected);
            }}
            onRequestIcon={() => setIconTarget(selected)}
            onRequestLeave={() => setLeaveTarget(selected)}
            onRequestRename={() => {
              setRenameValue(selected.name);
              setRenameTarget(selected);
            }}
          />
        ) : null}
      </div>

      <NewGroupDialog
        bridge={bridge}
        onCreated={(group) => {
          applyGroup(group);
          setSelectedId(group.id);
          void refresh();
          go('chats', group.id);
        }}
        onOpenChange={setNewOpen}
        open={newOpen}
        peers={peers}
        toast={toast}
      />

      <ChangeIconDialog
        bridge={bridge}
        group={iconTarget}
        onOpenChange={(next) => {
          if (!next) setIconTarget(null);
        }}
        onUpdated={applyGroup}
        toast={toast}
      />

      <AddMembersDialog
        bridge={bridge}
        group={addTarget}
        onOpenChange={(next) => {
          if (!next) setAddTarget(null);
        }}
        onUpdated={applyGroup}
        peers={peers}
        toast={toast}
      />

      <FieldDialog
        busy={busy}
        description={text('renameDescription', 'The new name is signed and synced to every member.')}
        helper={text('nameHelper', 'Up to {{max}} characters.', { max: GROUP_NAME_MAX })}
        label={text('nameLabel', 'Group name')}
        maxLength={GROUP_NAME_MAX}
        onChange={setRenameValue}
        onOpenChange={(next) => {
          if (!next) setRenameTarget(null);
        }}
        onSubmit={() => void submitRename()}
        submitLabel={text('save', 'Save')}
        open={Boolean(renameTarget)}
        title={text('renameTitle', 'Rename group')}
        value={renameValue}
      />

      <FieldDialog
        busy={busy}
        description={text('descriptionDescription', 'What is this group for? Every member sees it.')}
        helper={text('descriptionHelper', 'Up to 140 characters.')}
        label={text('descriptionLabel', 'Description')}
        maxLength={140}
        onChange={setDescriptionValue}
        onOpenChange={(next) => {
          if (!next) setDescriptionTarget(null);
        }}
        onSubmit={() => void submitDescription()}
        submitLabel={text('save', 'Save')}
        open={Boolean(descriptionTarget)}
        title={text('descriptionTitle', 'Edit description')}
        value={descriptionValue}
      />

      <Dialog
        description={text('leaveDescription', 'You will stop receiving messages from {{name}} on this device.', { name: leaveTarget?.name ?? '' })}
        footer={
          <>
            <Button onClick={() => setLeaveTarget(null)} variant="secondary">
              {text('cancel', 'Cancel')}
            </Button>
            <Button icon="logOut" loading={busy} onClick={() => void confirmLeave()} variant="danger">
              {text('leaveGroup', 'Leave Group')}
            </Button>
          </>
        }
        onOpenChange={(next) => {
          if (!next) setLeaveTarget(null);
        }}
        open={Boolean(leaveTarget)}
        title={text('leaveTitle', 'Leave this group?')}
      >
        <Banner icon="alertCircle" message={text('leaveWarning', 'Your copy of the history stays on this device. Rejoining needs a new invite from an admin.')} tone="warning" />
      </Dialog>

      <ContextMenu
        anchor={menu ? { x: menu.x, y: menu.y } : null}
        items={menu?.entries ?? []}
        label={menu?.label ?? text('menuLabel', 'Group actions')}
        onClose={() => setMenu(null)}
        open={Boolean(menu)}
      />
    </div>
  );
}

export default GroupsScreen;
