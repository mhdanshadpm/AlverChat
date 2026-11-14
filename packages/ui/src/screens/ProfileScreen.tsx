import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type MouseEvent,
} from 'react';
import type {
  AppSettings,
  Attachment,
  ChatMessage,
  Peer,
  PlatformBridge,
} from '../../../bridge/src';
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
  PresenceDot,
  Select,
  Skeleton,
  Slider,
  Toggle,
  Tooltip,
  avatarPaletteFor,
  contextMenuItems,
  cx,
  fileKindFor,
  formatFileSize,
  initialsFor,
  type AvatarPaletteColor,
  type ContextMenuEntry,
  type FileGlyph,
  type IconName,
  type PresenceState,
} from '../components';
import { ns, registerNamespace } from '../i18n';
import profileCatalog from '../i18n/en/profile';
import { ScreenHeader, type ScreenProps, type ToastTone } from './shared';
import {
  AVATAR_MAX_BYTES,
  AVATAR_PIXELS,
  AVATAR_PREVIEW_SIZES,
  AVATAR_ZOOM_MAX,
  AVATAR_ZOOM_MIN,
  avatarFromDataUrl,
  droppedImage,
  isPaletteColor,
  normalizeDegrees,
  pickImageFile,
  prepareAvatarDataUrl,
  validateAvatarFile,
  zoomFromPercent,
  type AvatarFileRejection,
} from './photoPipeline';
import './ProfileScreen.css';

registerNamespace('profile', profileCatalog);
const text = ns('profile');

/* ========================================================================== *
 * Types + pure helpers
 * ========================================================================== */

/** The four presence states on the profile board, in board order. */
const STATUS_VALUES = ['Available', 'Busy', 'In a meeting', 'Away'] as const;

const CUSTOM_STATUS_MAX = 100;
const DISPLAY_NAME_MAX = 40;

/** Where each preview size is used, straight off the board. */
const PREVIEW_CONTEXT = ['Chat, Profile', 'Contact List', 'Minimized'];

const GLYPH_ICON: Record<FileGlyph, IconName> = {
  archive: 'archive',
  audio: 'fileAudio',
  doc: 'fileText',
  media: 'image',
  pdf: 'fileText',
  sheet: 'fileText',
  file: 'file',
};

type MenuState = { entries: ContextMenuEntry[]; label: string; x: number; y: number };

export type MediaItem = {
  attachmentId: string | null;
  glyph: IconName;
  id: string;
  isMedia: boolean;
  name: string;
  sender: string;
  size: number | null;
  time: string;
  url: string | null;
};

function failure(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/** Human copy for the pre-decode gate; nothing is decoded before this passes. */
function rejectionMessage(reason: AvatarFileRejection): string {
  if (reason === 'type') return text('errorType', 'Only PNG, JPEG or WebP pictures can be used.');
  if (reason === 'size') {
    return text('errorSize', 'That picture is bigger than {{max}} MB.', { max: AVATAR_MAX_BYTES / (1024 * 1024) });
  }
  return text('errorEmpty', 'That file is empty.');
}

/** Maps a stored presence string onto the four dot states. */
export function presenceFor(status: string): PresenceState {
  const value = (status ?? '').toLowerCase();
  if (value.includes('offline')) return 'offline';
  if (value.includes('meeting') || value.includes('busy') || value.includes('dnd')) return 'busy';
  if (value.includes('away')) return 'away';
  return 'online';
}

export function statusLabel(status: string): string {
  switch (status) {
    case 'Busy':
      return text('statusBusy', 'Busy');
    case 'In a meeting':
      return text('statusMeeting', 'In a meeting');
    case 'Away':
      return text('statusAway', 'Away');
    case 'Available':
      return text('statusAvailable', 'Available');
    default:
      return status;
  }
}

/**
 * Device ids arrive from the network as long opaque strings (`a1b2c3d4e5f60718`);
 * the friendly `peer-*` ids our own list mints are not fingerprints.
 */
export function looksLikeDeviceId(id: string): boolean {
  if (!id) return false;
  if (/^(peer|chat|group|user|me|self|everyone)[-_:/]/i.test(id)) return false;
  return /^[a-z0-9][a-z0-9-_:.]{15,}$/i.test(id);
}

/** Groups an opaque id into readable 4-character blocks. */
export function formatFingerprint(value: string): string {
  const clean = value.replace(/[^0-9a-z]/gi, '').toUpperCase();
  return (clean.match(/.{1,4}/g) ?? []).slice(0, 6).join(' ');
}

/** One file message plus its transfer record, if the peer announced one. */
export function mediaItemOf(message: ChatMessage, attachments: readonly Attachment[]): MediaItem {
  const name = message.fileName ?? message.text;
  const kind = fileKindFor(name);
  const match = attachments.find((attachment) => attachment.messageId === message.id) ?? null;
  return {
    attachmentId: match?.id ?? null,
    glyph: GLYPH_ICON[kind] ?? 'file',
    id: message.id,
    isMedia: kind === 'media',
    name,
    sender: message.sender,
    size: match?.bytes ?? null,
    time: message.time,
    url: match?.previewUrl ?? null,
  };
}

/** Everything shared as a file, oldest message first, straight off the bridge. */
export function filesOf(messages: readonly ChatMessage[], attachments: readonly Attachment[]): MediaItem[] {
  return messages.filter((message) => message.kind === 'file').map((message) => mediaItemOf(message, attachments));
}

function avatarColorFor(value: string, seed: string): AvatarPaletteColor {
  return isPaletteColor(value) ? value : avatarPaletteFor(seed);
}

/* ========================================================================== *
 * My Profile card (spec 7.7)
 * ========================================================================== */

export function MyProfileCard({
  avatar,
  identity,
  loading,
  onEditPhoto,
  patchSettings,
  settings,
  toast,
}: {
  avatar: string | null;
  identity: ScreenProps['identity'];
  loading: boolean;
  onEditPhoto: () => void;
  patchSettings: (patch: Partial<AppSettings>) => void;
  settings: AppSettings;
  toast: (message: string, tone?: ToastTone) => void;
}) {
  const [statusDraft, setStatusDraft] = useState(settings.customStatus);
  const [nameOpen, setNameOpen] = useState(false);
  const [nameValue, setNameValue] = useState(settings.displayName);
  const [nameError, setNameError] = useState<string | null>(null);

  useEffect(() => setStatusDraft(settings.customStatus), [settings.customStatus]);

  const shownName = settings.displayName.trim() || text('you', 'You');
  const color = avatarColorFor(settings.avatarColor, shownName);
  const dirty = statusDraft !== settings.customStatus;

  function saveStatus() {
    patchSettings({ customStatus: statusDraft });
    toast(text('statusSaved', 'Custom status updated.'), 'success');
  }

  function submitName() {
    const next = nameValue.trim();
    if (next.length === 0) {
      setNameError(text('nameRequired', 'Enter a name your contacts will recognise.'));
      return;
    }
    patchSettings({ displayName: next });
    setNameOpen(false);
    toast(text('nameSaved', 'Display name saved.'), 'success');
  }

  return (
    <section className="prf-card prf-card--me">
      <header className="prf-card__head">
        <h3 className="prf-card__title">{text('myProfile', 'My Profile')}</h3>
        <p className="prf-card__hint">{text('myProfileHint', 'This is the card every peer on your network sees.')}</p>
      </header>

      <div className="prf-identity-block">
        {loading ? (
          <Skeleton variant="avatar" />
        ) : (
          <span className="prf-avatar-wrap">
            <Avatar
              color={color}
              initials={initialsFor(shownName)}
              name={shownName}
              presence={presenceFor(settings.presence)}
              size="lg"
              src={avatar ?? undefined}
            />
            <IconButton
              className="prf-avatar-cam"
              label={text('editPhoto', 'Edit profile photo')}
              name="camera"
              onClick={onEditPhoto}
              size="sm"
              tooltip={text('editPhoto', 'Edit profile photo')}
              variant="soft"
            />
          </span>
        )}

        <div className="prf-name-row">
          <h4 className="prf-name">{shownName}</h4>
          <IconButton
            label={text('editName', 'Change your display name')}
            name="pencil"
            onClick={() => {
              setNameError(null);
              setNameValue(settings.displayName);
              setNameOpen(true);
            }}
            size="sm"
          />
        </div>
      </div>

      <div className="prf-field">
        <span className="prf-status-head">
          <PresenceDot state={presenceFor(settings.presence)} />
          <span className="prf-field__label">{text('status', 'Status')}</span>
        </span>
        <Select
          aria-label={text('status', 'Status')}
          block
          onChange={(event) => patchSettings({ presence: event.target.value })}
          options={STATUS_VALUES.map((value) => ({ label: statusLabel(value), value }))}
          value={STATUS_VALUES.includes(settings.presence as (typeof STATUS_VALUES)[number]) ? settings.presence : 'Available'}
        />
      </div>

      <Input
        hint={text('customStatusHint', 'Shown under your name in chat lists and in the dock.')}
        icon="messageCircle"
        label={text('customStatus', 'Custom Status')}
        maxLength={CUSTOM_STATUS_MAX}
        onChange={(event) => setStatusDraft(event.target.value)}
        onTrailingClick={() => setStatusDraft('')}
        showCount
        trailingIcon={statusDraft.length > 0 ? 'x' : undefined}
        trailingLabel={text('clearStatus', 'Clear your custom status')}
        value={statusDraft}
      />

      <div className="prf-field">
        <span className="prf-field__label">{text('avatarColor', 'Avatar colour')}</span>
        <div aria-label={text('avatarColorGroup', 'Initials avatar colour')} className="prf-palette" role="group">
          {AVATAR_PALETTE.map((option) => (
            <Button
              aria-label={text('paletteColor', '{{color}} avatar colour', { color: option })}
              aria-pressed={option === color}
              className={cx('prf-swatch', `prf-swatch--${option}`, option === color && 'prf-swatch--on')}
              key={option}
              onClick={() => patchSettings({ avatarColor: option })}
              size="sm"
              variant="ghost"
            />
          ))}
        </div>
      </div>

      <div className="prf-card__actions">
        <Button disabled={!dirty} icon="check" onClick={saveStatus}>
          {text('saveChanges', 'Save Changes')}
        </Button>
        <Button disabled={!dirty} onClick={() => setStatusDraft(settings.customStatus)} variant="secondary">
          {text('cancel', 'Cancel')}
        </Button>
      </div>

      <ListGroup className="prf-identity-rows">
        <ListRow
          icon="shieldCheck"
          hint={identity?.fingerprint ?? text('identityPending', 'Waiting for this device to announce itself.')}
          title={text('identity', 'Your network fingerprint')}
        />
        <ListRow
          icon="globe"
          hint={identity?.peerId ?? text('identityPending', 'Waiting for this device to announce itself.')}
          title={text('peerId', 'Your peer id')}
        />
      </ListGroup>

      <Dialog
        description={text('nameDialogHint', 'Peers see this name on their LAN, in bubbles and in group rosters.')}
        onOpenChange={setNameOpen}
        open={nameOpen}
        title={text('editNameTitle', 'Edit display name')}
        footer={
          <>
            <Button onClick={() => setNameOpen(false)} variant="secondary">
              {text('cancel', 'Cancel')}
            </Button>
            <Button icon="check" onClick={submitName}>
              {text('save', 'Save')}
            </Button>
          </>
        }
      >
        <Input
          error={nameError ?? undefined}
          label={text('displayName', 'Display name')}
          maxLength={DISPLAY_NAME_MAX}
          onChange={(event) => setNameValue(event.target.value)}
          showCount
          value={nameValue}
        />
      </Dialog>
    </section>
  );
}

/* ========================================================================== *
 * Edit Photo — crop, zoom, rotate, live 56/40/24 previews, remove
 * ========================================================================== */

export function CropPreview({
  rotate,
  size,
  src,
  zoom,
}: {
  rotate: number;
  size: number;
  src: string | null;
  zoom: number;
}) {
  const style = { '--prf-crop': `${size}px` } as CSSProperties;
  const imageStyle = { transform: `scale(${zoom / 100}) rotate(${rotate}deg)` } as CSSProperties;
  return (
    <span className={cx('prf-crop', size >= 200 && 'prf-crop--canvas')} style={style}>
      {src ? <img alt="" className="prf-crop__img" src={src} style={imageStyle} /> : null}
      {size >= 200 ? <span aria-hidden className="prf-crop__grid" /> : null}
    </span>
  );
}

export function EditPhotoDialog({
  bridge,
  hasAvatar,
  onOpenChange,
  onStored,
  open,
  toast,
}: {
  bridge: PlatformBridge;
  hasAvatar: boolean;
  onOpenChange: (open: boolean) => void;
  onStored: (avatar: string | null) => void;
  open: boolean;
  toast: (message: string, tone?: ToastTone) => void;
}) {
  const [file, setFile] = useState<File | null>(null);
  const [preview, setPreview] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [mode, setMode] = useState<'crop' | 'upload'>('crop');
  const [zoom, setZoom] = useState(AVATAR_ZOOM_MIN);
  const [rotate, setRotate] = useState(0);
  const [busy, setBusy] = useState(false);
  const [dropping, setDropping] = useState(false);
  const urlRef = useRef<string | null>(null);

  const release = useCallback(() => {
    if (!urlRef.current) return;
    URL.revokeObjectURL(urlRef.current);
    urlRef.current = null;
  }, []);

  useEffect(() => release, [release]);

  const takeFile = useCallback(
    (next: File | null) => {
      if (!next) return;
      /* Gate first: type and size are verified before a single pixel is decoded. */
      const check = validateAvatarFile(next);
      if (!check.ok) {
        release();
        setFile(null);
        setPreview(null);
        setError(rejectionMessage(check.reason));
        return;
      }
      release();
      const url = URL.createObjectURL(next);
      urlRef.current = url;
      setFile(next);
      setPreview(url);
      setError(null);
      setMode('crop');
      setZoom(AVATAR_ZOOM_MIN);
      setRotate(0);
    },
    [release],
  );

  async function save() {
    if (!file) return;
    setBusy(true);
    const result = await prepareAvatarDataUrl(file, { rotate, zoom: zoomFromPercent(zoom) });
    if (!result.ok) {
      setBusy(false);
      setError(rejectionMessage(result.reason));
      return;
    }
    try {
      const stored = await bridge.setAvatar(result.dataUrl);
      onStored(avatarFromDataUrl(stored));
      toast(
        text('photoSaved', 'Photo saved — {{px}} px, EXIF stripped, {{size}}.', {
          px: AVATAR_PIXELS,
          size: formatFileSize(result.bytes),
        }),
        'success',
      );
      onOpenChange(false);
    } catch (cause) {
      toast(failure(cause), 'danger');
    } finally {
      setBusy(false);
    }
  }

  async function remove() {
    setBusy(true);
    try {
      await bridge.setAvatar(null);
      onStored(null);
      release();
      setFile(null);
      setPreview(null);
      setError(null);
      toast(text('photoRemoved', 'Photo removed — your initials are used again.'), 'success');
      onOpenChange(false);
    } catch (cause) {
      toast(failure(cause), 'danger');
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog
      className="prf-dialog--photo"
      description={text(
        'photoHint',
        'Pictures are resized to {{px}} px and re-encoded as JPEG, which drops EXIF — including GPS.',
        { px: AVATAR_PIXELS },
      )}
      footerLayout="end"
      onOpenChange={onOpenChange}
      open={open}
      size="xl"
      title={text('editPhotoTitle', 'Edit Photo')}
      footer={
        <>
          <Button onClick={() => onOpenChange(false)} variant="secondary">
            {text('cancel', 'Cancel')}
          </Button>
          <Button disabled={!file} icon="check" loading={busy} onClick={() => void save()}>
            {text('savePhoto', 'Save Photo')}
          </Button>
        </>
      }
    >
      <div className="prf-photo">
        <ListGroup className="prf-photo__rail">
          <ListRow
            active={mode === 'crop'}
            icon="move"
            onClick={() => setMode('crop')}
            title={text('cropAdjust', 'Crop & Adjust')}
          />
          <ListRow
            active={mode === 'upload'}
            icon="cloudUpload"
            onClick={() => void pickImageFile().then(takeFile)}
            title={text('uploadNew', 'Upload New')}
          />
          <ListRow
            className="prf-row--danger"
            disabled={!hasAvatar}
            icon="trash"
            onClick={() => void remove()}
            title={text('removePhoto', 'Remove Photo')}
          />
        </ListGroup>

        <div className="prf-photo__stage">
          {error ? (
            <Banner message={error} tone="danger" title={text('photoRejected', 'That picture was not accepted')} />
          ) : null}

          {preview && mode === 'crop' ? (
            <>
              <CropPreview rotate={rotate} size={232} src={preview} zoom={zoom} />
              <div className="prf-zoom">
                <Slider
                  format={(value) => `${value}%`}
                  label={text('zoom', 'Zoom')}
                  max={AVATAR_ZOOM_MAX}
                  min={AVATAR_ZOOM_MIN}
                  onChange={setZoom}
                  value={zoom}
                />
                <div className="prf-zoom__buttons">
                  <IconButton
                    label={text('rotateLeft', 'Rotate left')}
                    name="rotateCcw"
                    onClick={() => setRotate((current) => normalizeDegrees(current - 90))}
                    variant="soft"
                  />
                  <IconButton
                    label={text('rotateRight', 'Rotate right')}
                    name="rotateCw"
                    onClick={() => setRotate((current) => normalizeDegrees(current + 90))}
                    variant="soft"
                  />
                  <IconButton
                    disabled={zoom === AVATAR_ZOOM_MIN && rotate === 0}
                    label={text('resetAdjust', 'Reset crop')}
                    name="undo"
                    onClick={() => {
                      setZoom(AVATAR_ZOOM_MIN);
                      setRotate(0);
                    }}
                    variant="soft"
                  />
                </div>
              </div>
            </>
          ) : (
            <div
              aria-label={text('dropHint', 'Drag a picture here')}
              className={cx('prf-drop', dropping && 'prf-drop--on')}
              onDragLeave={() => setDropping(false)}
              onDragOver={(event) => {
                event.preventDefault();
                setDropping(true);
              }}
              onDrop={(event) => {
                event.preventDefault();
                setDropping(false);
                takeFile(droppedImage(event));
              }}
              role="presentation"
            >
              <Icon name="imagePlus" size={26} />
              <span>{text('dropTitle', 'Drop a picture here')}</span>
              <span className="prf-drop__note">
                {text('dropNote', 'PNG, JPEG or WebP up to {{max}} MB.', {
                  max: AVATAR_MAX_BYTES / (1024 * 1024),
                })}
              </span>
              <Button icon="upload" onClick={() => void pickImageFile().then(takeFile)} variant="secondary">
                {text('chooseFile', 'Choose photo…')}
              </Button>
            </div>
          )}
        </div>

        <div className="prf-previews">
          <span className="prf-previews__title">{text('preview', 'Preview')}</span>
          <span className="prf-previews__note">{text('previewNote', 'How it will look in chat')}</span>
          {AVATAR_PREVIEW_SIZES.map((size, index) => (
            <div className="prf-preview" key={size}>
              <CropPreview rotate={rotate} size={size} src={preview} zoom={zoom} />
              <span className="prf-preview__text">
                <strong>{text('previewPx', '{{size}} px', { size })}</strong>
                <em>{text(`previewUse${index}`, PREVIEW_CONTEXT[index])}</em>
              </span>
            </div>
          ))}
        </div>
      </div>
    </Dialog>
  );
}

/* ========================================================================== *
 * Contact Profile
 * ========================================================================== */

export function ContactProfile({
  contact,
  loading,
  mediaCount,
  muted,
  onCall,
  onClearRequest,
  onMenu,
  onMessage,
  onMute,
  onOpenFiles,
  onOpenMedia,
  fileCount,
}: {
  contact: Peer;
  loading: boolean;
  mediaCount: number;
  muted: boolean;
  onCall: (kind: 'video' | 'voice') => void;
  onClearRequest: () => void;
  onMenu: (event: MouseEvent<HTMLElement>, label: string, entries: ContextMenuEntry[]) => void;
  onMessage: () => void;
  onMute: (next: boolean) => void;
  onOpenFiles: () => void;
  onOpenMedia: () => void;
  fileCount: number;
}) {
  const color = avatarColorFor(contact.color, contact.name);
  const fingerprint = looksLikeDeviceId(contact.id) ? formatFingerprint(contact.id) : null;

  const entries: ContextMenuEntry[] = [
    ...contextMenuItems([
      { icon: 'messageCircle', id: 'message', label: text('message', 'Message'), onSelect: onMessage },
      { icon: 'phone', id: 'voice', label: text('voiceCall', 'Voice Call'), onSelect: () => onCall('voice') },
      { icon: 'video', id: 'video', label: text('videoCall', 'Video Call'), onSelect: () => onCall('video') },
    ]),
    { id: 'contact-sep', kind: 'separator' },
    ...contextMenuItems([
      {
        danger: true,
        disabled: loading,
        icon: 'trash',
        id: 'clear',
        label: text('clearChat', 'Clear chat'),
        onSelect: onClearRequest,
      },
    ]),
  ];

  return (
    <>
      <header className="prf-card__head prf-card__head--split">
        <div>
          <h3 className="prf-card__title">{text('contactProfile', 'Contact Profile')}</h3>
          <p className="prf-card__hint">{text('contactHint', 'Everything this peer announced on the local network.')}</p>
        </div>
        <IconButton
          label={text('moreActions', 'More actions for {{name}}', { name: contact.name })}
          name="moreHorizontal"
          onClick={(event) => onMenu(event, text('contactMenu', 'Contact actions'), entries)}
        />
      </header>

      <div className="prf-contact-head">
        <Avatar
          color={color}
          initials={contact.initials || initialsFor(contact.name)}
          name={contact.name}
          presence={contact.online ? presenceFor(contact.status) : 'offline'}
          size="lg"
        />
        <h4 className="prf-contact-name">{contact.name}</h4>
        <p className="prf-contact-status">
          {contact.online ? (
            <>
              <PresenceDot state={presenceFor(contact.status)} />
              <span>{statusLabel(contact.status) || text('statusAvailable', 'Available')}</span>
            </>
          ) : (
            <>
              <Icon name="clock" size={14} />
              <span>{text('lastSeen', 'Last seen {{time}}', { time: contact.time })}</span>
            </>
          )}
        </p>
      </div>

      {fingerprint ? (
        <ListGroup className="prf-identity-rows">
          <ListRow
            icon="shieldCheck"
            hint={text('fingerprintHint', 'Verified on this network segment — no server ever sees it.')}
            title={text('fingerprint', 'Local-network fingerprint')}
            trailing={<span className="prf-mono">{fingerprint}</span>}
          />
        </ListGroup>
      ) : null}

      <div className="prf-actions">
        <Button icon="messageCircle" onClick={onMessage}>
          {text('message', 'Message')}
        </Button>
        <Tooltip content={text('callsSoon', 'Voice and video calls ship in phase 6.')} wrap>
          <Button icon="phone" onClick={() => onCall('voice')} variant="secondary">
            {text('voiceCall', 'Voice Call')}
          </Button>
        </Tooltip>
        <Tooltip content={text('callsSoon', 'Voice and video calls ship in phase 6.')} wrap>
          <Button icon="video" onClick={() => onCall('video')} variant="secondary">
            {text('videoCall', 'Video Call')}
          </Button>
        </Tooltip>
      </div>

      {loading ? (
        <Skeleton lines={3} variant="text" />
      ) : (
        <ListGroup className="prf-shared">
          <ListRow
            chevron
            icon="image"
            onClick={onOpenMedia}
            title={text('sharedMedia', 'Shared Media')}
            trailing={<Badge tone="neutral">{text('itemsCount', '{{count}} items', { count: mediaCount })}</Badge>}
          />
          <ListRow
            chevron
            icon="fileText"
            onClick={onOpenFiles}
            title={text('files', 'Files')}
            trailing={<Badge tone="neutral">{text('itemsCount', '{{count}} items', { count: fileCount })}</Badge>}
          />
          <ListRow
            icon={muted ? 'bellOff' : 'bell'}
            title={text('muteNotifications', 'Mute Notifications')}
            trailing={
              <Toggle
                checked={muted}
                label={text('muteToggle', 'Mute notifications from {{name}}', { name: contact.name })}
                onChange={onMute}
              />
            }
          />
        </ListGroup>
      )}

      <Button block disabled={loading} icon="trash" onClick={onClearRequest} variant="danger">
        {text('clearChat', 'Clear chat')}
      </Button>
    </>
  );
}

/* ========================================================================== *
 * Fullscreen media viewer
 * ========================================================================== */

export function MediaViewer({
  index,
  items,
  onClose,
  onDownload,
  onIndex,
  title,
}: {
  index: number;
  items: readonly MediaItem[];
  onClose: () => void;
  onDownload: (item: MediaItem) => void;
  onIndex: (next: number) => void;
  title: string;
}) {
  const [info, setInfo] = useState(false);
  const [menu, setMenu] = useState<MenuState | null>(null);
  const closeRef = useRef<HTMLDivElement>(null);
  const current = items[index] ?? null;

  const step = useCallback(
    (delta: number) => {
      if (items.length === 0) return;
      onIndex((index + delta + items.length) % items.length);
    },
    [index, items.length, onIndex],
  );

  useEffect(() => {
    closeRef.current?.focus();
  }, []);

  useEffect(() => {
    function onKey(event: KeyboardEvent) {
      if (event.key === 'Escape') {
        event.preventDefault();
        onClose();
      } else if (event.key === 'ArrowRight') {
        event.preventDefault();
        step(1);
      } else if (event.key === 'ArrowLeft') {
        event.preventDefault();
        step(-1);
      }
    }
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose, step]);

  const openMenu = (event: MouseEvent<HTMLElement>, entries: ContextMenuEntry[]) => {
    const box = event.currentTarget.getBoundingClientRect();
    setMenu({ entries, label: text('viewerMenu', 'Photo actions'), x: box.left - 180, y: box.bottom + 6 });
  };

  return (
    <div
      aria-label={text('viewerLabel', 'Shared media from {{name}}', { name: title })}
      aria-modal="true"
      className="prf-viewer"
      ref={closeRef}
      role="dialog"
      tabIndex={-1}
    >
      <header className="prf-viewer__bar">
        <IconButton label={text('viewerClose', 'Close the viewer')} name="arrowLeft" onClick={onClose} />
        <span className="prf-viewer__counter">
          {text('viewerCounter', '{{current}} of {{total}}', { current: index + 1, total: items.length })}
        </span>
        <span className="prf-viewer__spacer" />
        <IconButton
          active={info}
          label={text('viewerInfo', 'Show file information')}
          name="info"
          onClick={() => setInfo((value) => !value)}
        />
        <IconButton
          disabled={!current}
          label={text('viewerDownload', 'Download {{name}}', { name: current?.name ?? '' })}
          name="download"
          onClick={() => (current ? onDownload(current) : undefined)}
        />
        <IconButton
          label={text('viewerMore', 'More photo actions')}
          name="moreHorizontal"
          onClick={(event) =>
            openMenu(
              event,
              [
                ...contextMenuItems([
                  {
                    disabled: !current,
                    icon: 'download',
                    id: 'download',
                    label: text('viewerDownloadItem', 'Download'),
                    onSelect: () => (current ? onDownload(current) : undefined),
                  },
                  {
                    checked: info,
                    icon: 'info',
                    id: 'info',
                    label: text('viewerInfo', 'Show file information'),
                    onSelect: () => setInfo((value) => !value),
                  },
                ]),
                { id: 'viewer-sep', kind: 'separator' },
                ...contextMenuItems([
                  { icon: 'x', id: 'close', label: text('viewerClose', 'Close the viewer'), onSelect: onClose },
                ]),
              ],
            )
          }
        />
      </header>

      <div className="prf-viewer__stage">
        <IconButton
          className="prf-viewer__nav prf-viewer__nav--prev"
          disabled={items.length < 2}
          label={text('viewerPrev', 'Previous item')}
          name="chevronLeft"
          onClick={() => step(-1)}
          variant="soft"
        />
        {current?.url ? (
          <img alt={text('viewerAlt', '{{name}} shared by {{sender}}', { name: current.name, sender: current.sender })} className="prf-viewer__img" src={current.url} />
        ) : (
          <div className="prf-viewer__blank">
            <Icon name={current?.glyph ?? 'file'} size={44} />
            <strong>{current?.name ?? text('viewerEmpty', 'Nothing to show')}</strong>
            <span>
              {text('viewerNoPreview', 'The picture stays on {{name}}’s device until you download it.', {
                name: current?.sender ?? title,
              })}
            </span>
          </div>
        )}
        <IconButton
          className="prf-viewer__nav prf-viewer__nav--next"
          disabled={items.length < 2}
          label={text('viewerNext', 'Next item')}
          name="chevronRight"
          onClick={() => step(1)}
          variant="soft"
        />
      </div>

      {info && current ? (
        <dl className="prf-viewer__info">
          <div>
            <dt>{text('viewerName', 'File')}</dt>
            <dd>{current.name}</dd>
          </div>
          <div>
            <dt>{text('viewerSender', 'Shared by')}</dt>
            <dd>{current.sender}</dd>
          </div>
          <div>
            <dt>{text('viewerTime', 'Time')}</dt>
            <dd>{current.time}</dd>
          </div>
          <div>
            <dt>{text('viewerSize', 'Size')}</dt>
            <dd>{current.size === null ? text('viewerUnknownSize', 'Not announced') : formatFileSize(current.size)}</dd>
          </div>
        </dl>
      ) : null}

      <footer className="prf-viewer__strip">
        {items.map((item, position) => (
          <Button
            aria-current={position === index ? 'true' : undefined}
            aria-label={text('viewerThumb', 'Show {{name}}', { name: item.name })}
            className={cx('prf-thumb', position === index && 'prf-thumb--on')}
            key={item.id}
            onClick={() => onIndex(position)}
            variant="ghost"
          >
            {item.url ? <img alt="" src={item.url} /> : <Icon name={item.glyph} size={18} />}
          </Button>
        ))}
      </footer>

      <ContextMenu
        anchor={menu ? { x: menu.x, y: menu.y } : null}
        items={menu?.entries ?? []}
        label={menu?.label ?? text('viewerMenu', 'Photo actions')}
        onClose={() => setMenu(null)}
        open={Boolean(menu)}
      />
    </div>
  );
}

/* ========================================================================== *
 * Screen
 * ========================================================================== */

export function ProfileScreen(props: ScreenProps) {
  const { activeChat, bridge, go, identity, peers, settings, patchSettings, toast } = props;

  const [avatar, setAvatar] = useState<string | null>(null);
  const [avatarLoading, setAvatarLoading] = useState(true);
  const [photoOpen, setPhotoOpen] = useState(false);
  const [contactId, setContactId] = useState<string | null>(null);
  const [messages, setMessages] = useState<readonly ChatMessage[]>([]);
  const [attachments, setAttachments] = useState<readonly Attachment[]>([]);
  const [messagesLoading, setMessagesLoading] = useState(false);
  const [clearOpen, setClearOpen] = useState(false);
  const [clearBusy, setClearBusy] = useState(false);
  const [viewer, setViewer] = useState<{ index: number; items: readonly MediaItem[] } | null>(null);
  const [menu, setMenu] = useState<MenuState | null>(null);

  const people = useMemo(() => peers.filter((peer) => !peer.group), [peers]);

  const contact = useMemo(() => {
    const wanted = contactId ?? activeChat;
    return people.find((peer) => peer.id === wanted) ?? people[0] ?? null;
  }, [activeChat, contactId, people]);

  useEffect(() => {
    let live = true;
    setAvatarLoading(true);
    void bridge
      .getAvatar()
      .then((stored) => {
        if (!live) return;
        setAvatar(avatarFromDataUrl(stored));
        setAvatarLoading(false);
      })
      .catch((cause) => {
        if (!live) return;
        setAvatarLoading(false);
        toast(failure(cause), 'danger');
      });
    return () => {
      live = false;
    };
  }, [bridge, toast]);

  useEffect(() => {
    let live = true;
    if (!contact) {
      setMessages([]);
      setAttachments([]);
      return;
    }
    setMessagesLoading(true);
    void Promise.all([bridge.listMessages(contact.id), bridge.attachmentsFor(contact.id)])
      .then(([list, files]) => {
        if (!live) return;
        setMessages(list);
        setAttachments(files);
        setMessagesLoading(false);
      })
      .catch((cause) => {
        if (!live) return;
        setMessages([]);
        setAttachments([]);
        setMessagesLoading(false);
        toast(failure(cause), 'danger');
      });
    return () => {
      live = false;
    };
  }, [bridge, contact, toast]);

  const files = useMemo(() => filesOf(messages, attachments), [attachments, messages]);
  const mediaItems = useMemo(() => files.filter((item) => item.isMedia), [files]);
  const docItems = useMemo(() => files.filter((item) => !item.isMedia), [files]);

  const muted = contact ? settings.mutedChats.includes(contact.id) || contact.muted === true : false;

  const openMenu = useCallback((event: MouseEvent<HTMLElement>, label: string, entries: ContextMenuEntry[]) => {
    const box = event.currentTarget.getBoundingClientRect();
    setMenu({ entries, label, x: box.left - 160, y: box.bottom + 6 });
  }, []);

  /**
   * Dial out through the shared call controller; the kind names are the
   * profile screen's "voice"/"video" while the bridge uses "audio"/"video".
   */
  const startCall = useCallback(
    (peer: Peer, kind: 'video' | 'voice') => {
      if (peer.group) {
        toast(text('callsGroupsLater', 'Group calls are not available yet.'), 'neutral');
        return;
      }
      if (!props.onCall) {
        toast(text('callsUnavailable', 'Calls are unavailable on this device.'), 'neutral');
        return;
      }
      props.onCall(peer.id, kind === 'voice' ? 'audio' : 'video');
    },
    [props, toast],
  );

  const toggleMute = useCallback(
    async (peer: Peer, next: boolean) => {
      try {
        const list = await bridge.setMutedChat(peer.id, next);
        patchSettings({ mutedChats: list });
        toast(
          next
            ? text('mutedToast', '{{name}} is muted on this device.', { name: peer.name })
            : text('unmutedToast', '{{name}} is no longer muted.', { name: peer.name }),
          'success',
        );
      } catch (cause) {
        toast(failure(cause), 'danger');
      }
    },
    [bridge, patchSettings, toast],
  );

  async function confirmClear() {
    if (!contact) return;
    const name = contact.name;
    setClearBusy(true);
    try {
      await bridge.clearChat(contact.id);
      setMessages([]);
      setAttachments([]);
      setViewer(null);
      setClearOpen(false);
      toast(text('clearedToast', 'Conversation with {{name}} cleared on this device.', { name }), 'success');
    } catch (cause) {
      toast(failure(cause), 'danger');
    } finally {
      setClearBusy(false);
    }
  }

  const download = useCallback(
    async (item: MediaItem) => {
      if (!item.attachmentId) {
        toast(text('downloadMissing', '“{{name}}” is not on this device yet.', { name: item.name }), 'neutral');
        return;
      }
      try {
        const path = await bridge.openAttachment(item.attachmentId);
        if (!path) {
          toast(
            text('downloadPending', '“{{name}}” has not finished transferring on this device.', { name: item.name }),
            'neutral',
          );
          return;
        }
        await bridge.revealPath(path);
        toast(text('downloadedToast', 'Revealed “{{name}}” in your downloads.', { name: item.name }), 'success');
      } catch (cause) {
        toast(failure(cause), 'danger');
      }
    },
    [bridge, toast],
  );

  function openCollection(kind: 'files' | 'media') {
    const items = kind === 'media' ? mediaItems : files;
    if (items.length === 0) {
      toast(
        kind === 'media'
          ? text('noMediaYet', 'No pictures or videos have been shared here yet.')
          : text('noFilesYet', 'No files have been shared here yet.'),
        'neutral',
      );
      return;
    }
    setViewer({ index: 0, items });
  }

  return (
    <div className="prf-screen">
      <ScreenHeader
        actions={
          <Button icon="camera" onClick={() => setPhotoOpen(true)}>
            {text('editPhotoAction', 'Edit photo')}
          </Button>
        }
        description={text('description', 'Your card, your photo and the peers you can reach on this network.')}
        eyebrow={text('eyebrow', 'YOU')}
        title={text('title', 'Profile')}
      />

      <div className="prf-body">
        <MyProfileCard
          avatar={avatar}
          identity={identity}
          loading={avatarLoading}
          onEditPhoto={() => setPhotoOpen(true)}
          patchSettings={patchSettings}
          settings={settings}
          toast={toast}
        />

        <section className="prf-card prf-card--contact">
          {people.length === 0 ? (
            <EmptyState
              icon="user"
              message={text('noContactMessage', 'AlverChat has not found anyone on this network yet.')}
              title={text('noContactTitle', 'No contact to show')}
            />
          ) : (
            <>
              <div className="prf-chooser">
                <Select
                  aria-label={text('chooseContact', 'Choose a contact')}
                  block
                  onChange={(event) => setContactId(event.target.value)}
                  options={people.map((peer) => ({ label: peer.name, value: peer.id }))}
                  value={contact?.id ?? ''}
                />
                <IconButton label={text('closeContact', 'Back to the chat list')} name="x" onClick={() => setContactId(null)} />
              </div>
              {contact ? (
                <ContactProfile
                  contact={contact}
                  fileCount={docItems.length}
                  loading={messagesLoading}
                  mediaCount={mediaItems.length}
                  muted={muted}
                  onCall={(kind) => startCall(contact, kind)}
                  onClearRequest={() => setClearOpen(true)}
                  onMenu={openMenu}
                  onMessage={() => go('chats', contact.id)}
                  onMute={(next) => void toggleMute(contact, next)}
                  onOpenFiles={() => openCollection('files')}
                  onOpenMedia={() => openCollection('media')}
                />
              ) : null}
            </>
          )}
        </section>
      </div>

      <EditPhotoDialog
        bridge={bridge}
        hasAvatar={Boolean(avatar)}
        onOpenChange={setPhotoOpen}
        onStored={setAvatar}
        open={photoOpen}
        toast={toast}
      />

      {viewer && viewer.items.length > 0 ? (
        <MediaViewer
          index={Math.min(viewer.index, viewer.items.length - 1)}
          items={viewer.items}
          onClose={() => setViewer(null)}
          onDownload={(item) => void download(item)}
          onIndex={(next) => setViewer((current) => (current ? { ...current, index: next } : current))}
          title={contact?.name ?? text('profile', 'Profile')}
        />
      ) : null}

      <Dialog
        description={text(
          'clearDescription',
          'Every message in this conversation is deleted from this device. The other peer keeps their copy.',
        )}
        onOpenChange={setClearOpen}
        open={clearOpen}
        size="sm"
        title={text('clearTitle', 'Clear this conversation?')}
        footer={
          <>
            <Button onClick={() => setClearOpen(false)} variant="secondary">
              {text('cancel', 'Cancel')}
            </Button>
            <Button icon="trash" loading={clearBusy} onClick={() => void confirmClear()} variant="danger">
              {text('clearConfirm', 'Clear chat')}
            </Button>
          </>
        }
      >
        <Banner
          message={text('clearWarning', 'This cannot be undone from the profile screen.')}
          tone="warning"
          title={contact ? text('clearTarget', '{{name}}', { name: contact.name }) : undefined}
        />
      </Dialog>

      <ContextMenu
        anchor={menu ? { x: menu.x, y: menu.y } : null}
        items={menu?.entries ?? []}
        label={menu?.label ?? text('menu', 'Menu')}
        onClose={() => setMenu(null)}
        open={Boolean(menu)}
      />
    </div>
  );
}

export default ProfileScreen;
