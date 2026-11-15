import type { ReactNode } from 'react';
import { ns, registerNamespace } from '../i18n';
import chatCatalog from '../i18n/en/chat';
import { Icon, cx, type IconName } from './Icon';
import { ProgressBar } from './ProgressBar';
import { Waveform } from './Waveform';
import { Avatar, type AvatarSize } from './Avatar';
import {
  deliveryTicks,
  fileKindFor,
  fileTransferLabel,
  formatDuration,
  formatFileSize,
  groupReactions,
  reactionTooltip,
  transferPercent,
  type AvatarPaletteColor,
  type DeliveryState,
  type FileGlyph,
  type FileTransferState,
  type ReactionInput,
} from './logic';

registerNamespace('chat', chatCatalog);

// Not `text`: both MessageBubble and VoiceBody destructure a `text` prop, which would shadow
// the helper and turn an ask into a call on a string.
const say = ns('chat');

export type MessageVariant = 'text' | 'file' | 'image' | 'video' | 'voice' | 'deleted';

export type BubbleReply = { sender?: string; text?: string };

export type BubbleMedia = { src: string; alt?: string; duration?: string };

export type MessageBubbleProps = {
  variant?: MessageVariant;
  /** Own message: accent bubble, right-aligned, shows delivery ticks. */
  mine?: boolean;
  text?: string;
  /** Group sender name shown above the bubble, tinted per the palette. */
  sender?: string;
  senderColor?: AvatarPaletteColor;
  time?: string;
  delivery?: DeliveryState;
  edited?: boolean;
  /** Provenance chip: this row is a copy of something forwarded into the chat. */
  forwarded?: boolean;
  /** Chip copy. Screens pass the localised strings; the gallery keeps the English defaults. */
  forwardedLabel?: string;
  editedLabel?: string;
  /** Quote block for a reply. */
  reply?: BubbleReply;
  /** Raw reactions; collapsed into one chip per emoji. */
  reactions?: readonly ReactionInput[];
  onReaction?: (emoji: string) => void;
  /** Leading avatar node; defaults to a plain 24px Avatar for incoming messages. */
  avatar?: ReactNode;
  /** Hover action strip (reply / more). */
  actions?: ReactNode;
  /** Multi-select checkbox state. */
  selected?: boolean;
  onSelectedChange?: (selected: boolean) => void;
  onContextMenu?: (point: { x: number; y: number }) => void;

  /* file */
  fileName?: string;
  fileSize?: number;
  transfer?: FileTransferState;
  transferred?: number;
  totalSize?: number;
  onDownload?: () => void;
  onRetry?: () => void;
  onPause?: () => void;
  onResume?: () => void;
  onCancel?: () => void;

  /* image / video */
  media?: readonly BubbleMedia[];
  /** Opens the fullscreen viewer for the tapped item. */
  onOpenMedia?: (index: number) => void;

  /* voice */
  voiceSeconds?: number;
  waveformSeed?: string | number;
  playing?: boolean;
  playedSeconds?: number;
  onTogglePlay?: () => void;
  onSeek?: (fraction: number) => void;

  /** Collapse the avatar slot for consecutive messages from the same sender. */
  grouped?: boolean;
  avatarSize?: AvatarSize;
  className?: string;
};

const FILE_GLYPH: Record<FileGlyph, { icon: IconName; text: string }> = {
  pdf: { icon: 'fileText', text: 'PDF' },
  sheet: { icon: 'fileText', text: 'XLS' },
  doc: { icon: 'fileText', text: 'DOC' },
  media: { icon: 'image', text: 'IMG' },
  audio: { icon: 'fileAudio', text: 'AUD' },
  archive: { icon: 'archive', text: 'ZIP' },
  file: { icon: 'file', text: 'FILE' },
};

/**
 * One bubble for every message kind on the showcase boards: text, file (icon +
 * name + size + live transfer state), image, video (duration chip), voice note
 * (waveform + duration + play), deleted tombstone, forwarded label, edited tag,
 * reply quote, reaction chips with a who-reacted tooltip, and delivery ticks.
 */
export function MessageBubble(props: MessageBubbleProps) {
  const {
    variant = 'text',
    mine = false,
    text,
    sender,
    senderColor,
    time,
    delivery,
    edited = false,
    forwarded = false,
    forwardedLabel = 'Forwarded',
    editedLabel = '(edited)',
    reply,
    reactions,
    onReaction,
    avatar,
    actions,
    selected,
    onSelectedChange,
    onContextMenu,
    grouped = false,
    avatarSize = 24,
    className,
  } = props;

  const tick = deliveryTicks(delivery, mine);
  const chips = reactions && reactions.length > 0 ? groupReactions(reactions) : [];

  const body: ReactNode = (() => {
    switch (variant) {
      case 'deleted':
        return (
          <span className="ac-bubble__tombstone">
            <Icon name="ban" size={14} />
            {text ?? 'This message was deleted.'}
          </span>
        );
      case 'file':
        return <FileBody {...props} />;
      case 'voice':
        return <VoiceBody {...props} />;
      case 'image':
      case 'video':
        return <MediaBody video={variant === 'video'} {...props} />;
      case 'text':
      default:
        return <span className="ac-bubble__text">{text}</span>;
    }
  })();

  return (
    <div
      className={cx('ac-bubble__row', mine && 'ac-bubble__row--mine', className)}
      onContextMenu={
        onContextMenu
          ? (event) => {
              event.preventDefault();
              onContextMenu({ x: event.clientX, y: event.clientY });
            }
          : undefined
      }
    >
      {typeof selected === 'boolean' && onSelectedChange ? (
        <input
          aria-label={time ? say('selectMessageFrom', 'Select message from {{time}}', { time }) : say('selectMessageAria', 'Select message')}
          checked={selected}
          className="ac-bubble__check"
          onChange={(event) => onSelectedChange(event.target.checked)}
          type="checkbox"
        />
      ) : null}
      {!mine ? (
        <span className={cx('ac-bubble__avatar', grouped && 'ac-bubble__avatar--hidden')}>
          {avatar ?? <Avatar size={avatarSize} />}
        </span>
      ) : null}
      <div
        className={cx(
          'ac-bubble',
          mine && 'ac-bubble--mine',
          variant !== 'text' && `ac-bubble--${variant}`,
          (variant === 'image' || variant === 'video') && 'ac-bubble--media',
          variant === 'deleted' && 'ac-bubble--deleted',
        )}
      >
        {sender && !mine ? (
          <span
            className={cx('ac-bubble__sender', senderColor && `ac-bubble__sender--${senderColor}`)}
          >
            {sender}
          </span>
        ) : null}

        {forwarded ? (
          <span className="ac-bubble__forwarded">
            <Icon name="forward" size={13} /> {forwardedLabel}
          </span>
        ) : null}

        {reply ? (
          <span className="ac-bubble__quote">
            {reply.sender ? <span className="ac-bubble__quote-author">{reply.sender}</span> : null}
            <span className="ac-bubble__quote-text">{reply.text}</span>
          </span>
        ) : null}

        {body}

        <span className="ac-bubble__meta">
          {edited ? <span className="ac-bubble__edited">{editedLabel}</span> : null}
          {time ? <time>{time}</time> : null}
          {tick.icon ? (
            <span aria-label={tick.label} className="ac-bubble__ticks" role="img">
              <Icon name={tick.icon} size={13} spin={tick.icon === 'loader'} />
              {tick.count > 1 ? <Icon name="check" size={13} /> : null}
            </span>
          ) : null}
        </span>

        {chips.length > 0 ? (
          <span className="ac-bubble__reactions">
            {chips.map((chip) => (
              <button
                aria-label={
                  chip.count === 1
                    ? say('reactionChip', '{{count}} reaction: {{names}}', { count: chip.count, names: reactionTooltip(chip.names) })
                    : say('reactionChipMany', '{{count}} reactions: {{names}}', { count: chip.count, names: reactionTooltip(chip.names) })
                }
                className="ac-bubble__reaction"
                key={chip.emoji}
                onClick={() => onReaction?.(chip.emoji)}
                type="button"
              >
                <span aria-hidden className="ac-bubble__reaction-emoji">
                  {chip.emoji}
                </span>
                <span className="ac-bubble__reaction-count">{chip.count}</span>
                <span className="ac-bubble__reaction-tip">{reactionTooltip(chip.names)}</span>
              </button>
            ))}
          </span>
        ) : null}
      </div>
      {actions ? <span className="ac-bubble__actions">{actions}</span> : null}
    </div>
  );
}

/** Icon + name + size + progress line + transfer controls. */
function FileBody(props: MessageBubbleProps) {
  const {
    fileName,
    fileSize,
    transfer,
    transferred,
    totalSize,
    onDownload,
    onRetry,
    onPause,
    onResume,
    onCancel,
  } = props;
  const kind = fileKindFor(fileName ?? '');
  const glyph = FILE_GLYPH[kind];
  const showProgress =
    typeof transferred === 'number' && typeof totalSize === 'number' && transfer !== 'failed';

  return (
    <>
      <span className={cx('ac-bubble__file-icon', `ac-bubble__file-icon--${kind}`)}>
        <Icon name={glyph.icon} size={17} />
      </span>
      <span className="ac-bubble__file-body">
        <span className="ac-bubble__file-name">{fileName ?? 'File'}</span>
        <span className="ac-bubble__file-size">
          {typeof fileSize === 'number' ? formatFileSize(fileSize) : ''}
          {typeof transferred === 'number' && typeof totalSize === 'number'
            ? ` · ${transferPercent(transferred, totalSize)}%`
            : ''}
        </span>
        {transfer && transfer !== 'complete' ? (
          <span
            className={cx(
              'ac-bubble__file-state',
              transfer === 'failed' && 'ac-bubble__file-state--failed',
            )}
          >
            {fileTransferLabel(transfer, transferred, totalSize)}
          </span>
        ) : null}
        {showProgress ? (
          <ProgressBar
            ariaLabel={fileName ?? 'File transfer'}
            tone={transfer === 'paused' ? 'warning' : 'accent'}
            value={transferPercent(transferred ?? 0, totalSize ?? 1)}
          />
        ) : null}
      </span>
      <span className="ac-bubble__file-actions">
        {transfer === 'sending' && onPause ? (
          <MiniAction icon="pause" label={say('pauseTransfer', 'Pause transfer')} onClick={onPause} />
        ) : null}
        {transfer === 'sending' && onCancel ? (
          <MiniAction icon="x" label={say('cancelTransfer', 'Cancel transfer')} onClick={onCancel} />
        ) : null}
        {transfer === 'paused' && onResume ? (
          <MiniAction icon="play" label={say('resumeTransfer', 'Resume transfer')} onClick={onResume} />
        ) : null}
        {transfer === 'failed' && onRetry ? (
          <button className="ac-btn ac-btn--sm ac-btn--secondary" onClick={onRetry} type="button">
            {say('retry', 'Retry')}
          </button>
        ) : null}
        {transfer === 'complete' && onDownload ? (
          <MiniAction icon="download" label={say('downloadFile', 'Download {{name}}', { name: fileName ?? 'file' })} onClick={onDownload} />
        ) : null}
        {!transfer && onDownload ? (
          <MiniAction icon="download" label={say('downloadFile', 'Download {{name}}', { name: fileName ?? 'file' })} onClick={onDownload} />
        ) : null}
      </span>
    </>
  );
}

function MiniAction({
  icon,
  label,
  onClick,
}: {
  icon: IconName;
  label: string;
  onClick: () => void;
}) {
  return (
    <button aria-label={label} className="ac-search__clear" onClick={onClick} type="button">
      <Icon name={icon} size={14} />
    </button>
  );
}

/** Play button + waveform + remaining duration. */
function VoiceBody(props: MessageBubbleProps) {
  const { voiceSeconds = 0, playedSeconds = 0, playing = false, onTogglePlay, onSeek } = props;
  return (
    <span className="ac-bubble__voice">
      <button
        aria-label={playing ? say('pauseVoiceNote', 'Pause voice note') : say('playVoiceNote', 'Play voice note')}
        className="ac-bubble__voice-btn"
        onClick={onTogglePlay}
        type="button"
      >
        <Icon name={playing ? 'pause' : 'play'} size={14} />
      </button>
      <Waveform
        durationSeconds={voiceSeconds}
        onSeek={onSeek}
        positionSeconds={playedSeconds}
        playing={playing}
        seed={props.waveformSeed ?? props.text ?? 'voice'}
      />
      <span className="ac-bubble__duration">
        {formatDuration(playing ? voiceSeconds - playedSeconds : voiceSeconds)}
      </span>
    </span>
  );
}

/** Single tile for one item, 2x2 grid with a "+n" overlay for several. */
function MediaBody(props: MessageBubbleProps & { video: boolean }) {
  const { media = [], video, onOpenMedia } = props;
  if (media.length === 0) return null;

  if (media.length === 1) {
    const only = media[0];
    return (
      <MediaTile
        alt={only.alt}
        duration={only.duration ?? (video ? '00:00' : undefined)}
        onOpen={onOpenMedia ? () => onOpenMedia(0) : undefined}
        src={only.src}
        video={video}
      />
    );
  }

  const shown = media.slice(0, 4);
  const extra = media.length - shown.length;
  return (
    <span className="ac-bubble__grid">
      {shown.map((item, index) => (
        <button
          aria-label={say('openImageNumbered', 'Open image {{number}}', { number: index + 1 })}
          className="ac-bubble__grid-cell"
          key={`${item.src}-${index}`}
          onClick={onOpenMedia ? () => onOpenMedia(index) : undefined}
          type="button"
        >
          <img alt={item.alt ?? ''} src={item.src} />
          {item.duration ? <span className="ac-bubble__chip">{item.duration}</span> : null}
          {index === 3 && extra > 0 ? (
            <span className="ac-bubble__grid-more">+{extra}</span>
          ) : null}
        </button>
      ))}
    </span>
  );
}

function MediaTile({
  src,
  alt,
  duration,
  video,
  onOpen,
}: {
  src: string;
  alt?: string;
  duration?: string;
  video: boolean;
  onOpen?: () => void;
}) {
  return (
    <button
      aria-label={video ? say('playVideo', 'Play video') : say('openImage', 'Open image')}
      className={cx('ac-bubble__media', video && 'ac-bubble__media--video')}
      onClick={onOpen}
      type="button"
    >
      <img alt={alt ?? ''} className="ac-bubble__media-img" src={src} />
      {video ? (
        <span className="ac-bubble__play">
          <span className="ac-bubble__play-disc">
            <Icon name="play" size={20} />
          </span>
        </span>
      ) : null}
      {duration ? <span className="ac-bubble__chip">{duration}</span> : null}
    </button>
  );
}
