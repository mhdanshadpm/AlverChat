import { placeContextMenu, type MenuPoint } from '../menu';
import type { IconName } from './Icon';

/* ------------------------------------------------------------------ *
 * Pure, DOM-free logic for the shared component library.
 * Everything here is unit-testable under vitest `environment: 'node'`.
 * ------------------------------------------------------------------ */

/* ===== Avatar ===== */

/** The 8 initials colours from design/LAN_Chat_Profile___Avatar_Showcase.png. */
export type AvatarPaletteColor =
  | 'violet'
  | 'blue'
  | 'green'
  | 'peach'
  | 'yellow'
  | 'rose'
  | 'purple'
  | 'cyan';

export const AVATAR_PALETTE: readonly AvatarPaletteColor[] = [
  'violet',
  'blue',
  'green',
  'peach',
  'yellow',
  'rose',
  'purple',
  'cyan',
];

export type PresenceState = 'online' | 'away' | 'busy' | 'offline';

export const PRESENCE_STATES: readonly PresenceState[] = [
  'online',
  'away',
  'busy',
  'offline',
];

export type AvatarMember = {
  name?: string;
  src?: string;
  color?: AvatarPaletteColor;
  presence?: PresenceState;
};

/** First letters of the first two words, upper-cased. Empty input yields "?". */
export function initialsFor(name: string | null | undefined): string {
  const words = (name ?? '').trim().split(/\s+/).filter((word) => word.length > 0);
  if (words.length === 0) return '?';
  const first = Array.from(words[0])[0];
  if (words.length === 1) return (first ?? '').toUpperCase();
  const second = Array.from(words[1])[0];
  return `${first ?? ''}${second ?? ''}`.toUpperCase();
}

/** Stable seed -> palette slot, so a peer always gets the same fallback colour. */
export function avatarPaletteFor(seed: string | null | undefined): AvatarPaletteColor {
  const key = (seed ?? '').trim().toLowerCase();
  if (key.length === 0) return 'violet';
  let hash = 0;
  for (const character of key) {
    hash = (hash * 31 + (character.codePointAt(0) ?? 0)) % 997;
  }
  return AVATAR_PALETTE[hash % AVATAR_PALETTE.length];
}

/** Presence rank used to decide who is shown in a composite group avatar. */
const PRESENCE_RANK: Record<PresenceState, number> = {
  online: 0,
  away: 1,
  busy: 2,
  offline: 3,
};

export const MAX_COMPOSITE_MEMBERS = 4;

/**
 * Up to four members for the 2x2 cluster on the groups board.
 * Online members are preferred, then away, then busy, then offline;
 * ties keep the caller's original order.
 */
export function compositeMembers(
  members: readonly AvatarMember[],
  limit = MAX_COMPOSITE_MEMBERS,
): AvatarMember[] {
  return members
    .map((member, index) => ({ member, index }))
    .sort((a, b) => {
      const rankA = PRESENCE_RANK[a.member.presence ?? 'offline'];
      const rankB = PRESENCE_RANK[b.member.presence ?? 'offline'];
      return rankA === rankB ? a.index - b.index : rankA - rankB;
    })
    .slice(0, Math.max(0, limit))
    .map((entry) => entry.member);
}

/* ===== MessageBubble ===== */

export type DeliveryState = 'sending' | 'delivered' | 'read';
export type FileTransferState = 'sending' | 'paused' | 'failed' | 'complete';

export type TickSpec = {
  /** Key into the shared icon map, or null when nothing is drawn. */
  icon: IconName | null;
  /** Screen-reader label, empty when nothing is drawn. */
  label: string;
  /** Number of check glyphs (0 for the spinner). */
  count: number;
};

/**
 * Delivery ticks from the "Message delivery states" panel:
 * sending = spinner, delivered = one check, read = double check.
 * Incoming messages never show a tick.
 */
export function deliveryTicks(delivery: DeliveryState | undefined, mine: boolean): TickSpec {
  if (!mine) return { icon: null, label: '', count: 0 };
  switch (delivery) {
    case 'read':
      return { icon: 'checkCheck', label: 'Read', count: 2 };
    case 'delivered':
      return { icon: 'check', label: 'Delivered', count: 1 };
    case 'sending':
    default:
      return { icon: 'loader', label: 'Sending', count: 0 };
  }
}

/** Leading label for the file bubble, mirroring the "File transfer states" panel. */
export function fileTransferLabel(
  state: FileTransferState | undefined,
  transferredBytes?: number,
  totalBytes?: number,
): string {
  switch (state) {
    case 'paused':
      return 'Paused';
    case 'failed':
      return 'Failed';
    case 'complete':
      return 'Complete';
    default: {
      if (totalBytes && typeof transferredBytes === 'number') {
        return `Sending… ${transferPercent(transferredBytes, totalBytes)}%`;
      }
      return 'Sending…';
    }
  }
}

const BINARY_UNITS = ['B', 'KB', 'MB', 'GB', 'TB'];

/** 2516582 -> "2.4 MB" (binary units, one decimal between 1 KB and 100 units). */
export function formatFileSize(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return '0 B';
  if (bytes < 1024) return `${Math.round(bytes)} B`;
  let value = bytes / 1024;
  let unit = 1;
  while (value >= 1024 && unit < BINARY_UNITS.length - 1) {
    value = value / 1024;
    unit = unit + 1;
  }
  const rounded = value >= 100 || unit === 1 ? Math.round(value) : Number(value.toFixed(1));
  return `${rounded} ${BINARY_UNITS[unit]}`;
}

export function transferPercent(transferred: number, total: number): number {
  if (!Number.isFinite(total) || total <= 0) return 0;
  return clampPercent(Math.round((transferred / total) * 100));
}

export function clampPercent(value: number): number {
  if (!Number.isFinite(value)) return 0;
  return Math.min(100, Math.max(0, Math.round(value)));
}

/** "6.8 MB of 12.4 MB (55%)" as drawn on the file bubble. */
export function fileTransferDetail(transferred: number, total: number): string {
  return `${formatFileSize(transferred)} of ${formatFileSize(total)} (${transferPercent(transferred, total)}%)`;
}

export type FileGlyph = 'pdf' | 'sheet' | 'doc' | 'media' | 'audio' | 'archive' | 'file';

const FILE_GLYPHS: Record<string, FileGlyph> = {
  pdf: 'pdf',
  xls: 'sheet',
  xlsx: 'sheet',
  csv: 'sheet',
  doc: 'doc',
  docx: 'doc',
  txt: 'doc',
  md: 'doc',
  png: 'media',
  jpg: 'media',
  jpeg: 'media',
  gif: 'media',
  webp: 'media',
  heic: 'media',
  mp4: 'media',
  mov: 'media',
  mkv: 'media',
  webm: 'media',
  mp3: 'audio',
  wav: 'audio',
  m4a: 'audio',
  ogg: 'audio',
  flac: 'audio',
  aac: 'audio',
  zip: 'archive',
  tar: 'archive',
  gz: 'archive',
  rar: 'archive',
  '7z': 'archive',
};

/** Picks the tinted file glyph from the extension, matching the board colours. */
export function fileKindFor(fileName: string): FileGlyph {
  const dot = fileName.lastIndexOf('.');
  if (dot < 0 || dot === fileName.length - 1) return 'file';
  return FILE_GLYPHS[fileName.slice(dot + 1).toLowerCase()] ?? 'file';
}

/** 156 -> "02:36", 3733 -> "01:02:13". */
export function formatDuration(totalSeconds: number): string {
  const seconds = Math.max(0, Math.round(totalSeconds));
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  const rest = seconds % 60;
  const pad = (value: number) => String(value).padStart(2, '0');
  return hours > 0 ? `${pad(hours)}:${pad(minutes)}:${pad(rest)}` : `${pad(minutes)}:${pad(rest)}`;
}

export type ReactionInput = { emoji: string; by: string };
export type ReactionGroup = { emoji: string; count: number; names: string[] };

/** Collapse raw reactions into one chip per emoji, in first-seen order. */
export function groupReactions(reactions: readonly ReactionInput[]): ReactionGroup[] {
  const order: string[] = [];
  const buckets = new Map<string, string[]>();
  for (const reaction of reactions) {
    const names = buckets.get(reaction.emoji);
    if (!names) {
      buckets.set(reaction.emoji, [reaction.by]);
      order.push(reaction.emoji);
      continue;
    }
    if (!names.includes(reaction.by)) names.push(reaction.by);
  }
  return order.map((emoji) => {
    const names = buckets.get(emoji) ?? [];
    return { emoji, count: names.length, names };
  });
}

/** Hover tooltip text for a reaction chip: "Liyana, Arjun, Sarah". */
export function reactionTooltip(names: readonly string[], limit = 3): string {
  if (names.length <= limit) return names.join(', ');
  return `${names.slice(0, limit).join(', ')} +${names.length - limit}`;
}

/* ===== ContextMenu ===== */

export const MENU_WIDTH = 220;
export const MENU_ITEM_HEIGHT = 40;
export const MENU_PADDING = 12;
export const MENU_REACTION_ROW_HEIGHT = 38;
export const MENU_SEPARATOR_HEIGHT = 13;
export const MENU_EDGE_MARGIN = 8;
export const MENU_DEFAULT_HEIGHT = 396;

/** Same shape as `MenuPoint`, plus whether the menu had to move off the pointer. */
export type MenuPlacement = MenuPoint & { flipped: boolean };

/**
 * Edge-flip helper for the context menu.
 *
 * Delegates the clamping maths to `placeContextMenu` in packages/ui/src/menu.ts so
 * both helpers always agree, then reports whether the result left the pointer.
 */
export function placeMenu(
  x: number,
  y: number,
  viewportWidth: number,
  viewportHeight: number,
  menuWidth = MENU_WIDTH,
  menuHeight = MENU_DEFAULT_HEIGHT,
): MenuPlacement {
  const point = placeContextMenu(x, y, viewportWidth, viewportHeight, menuWidth, menuHeight);
  return { left: point.left, top: point.top, flipped: point.left !== x || point.top !== y };
}

/** Estimated content height so `placeMenu` can flip before the menu is measured. */
export function measureMenuHeight(
  itemCount: number,
  options: { reactionRow?: boolean; separators?: number } = {},
): number {
  const separators = Math.max(0, options.separators ?? 0);
  const reactionRow = options.reactionRow ? MENU_REACTION_ROW_HEIGHT : 0;
  return (
    MENU_PADDING +
    Math.max(0, itemCount) * MENU_ITEM_HEIGHT +
    separators * MENU_SEPARATOR_HEIGHT +
    reactionRow
  );
}

/* ===== Badge ===== */

/** 128 with max 99 -> "99+". */
export function formatCount(count: number, max = 99): string {
  if (!Number.isFinite(count) || count <= 0) return '0';
  if (count > max) return `${max}+`;
  return String(Math.round(count));
}

/* ===== Waveform ===== */

/**
 * Deterministic pseudo-random amplitudes in [0.12, 1] so a voice note always
 * draws the same shape. No Math.random, so it is stable in snapshots and tests.
 */
export function waveformBars(seed: string | number, count = 32): number[] {
  const safeCount = Math.max(1, Math.round(count));
  let state = typeof seed === 'number' ? Math.abs(Math.round(seed)) : hashString(seed);
  const bars: number[] = [];
  for (let index = 0; index < safeCount; index += 1) {
    state = (state * 1664525 + 1013904223) % 4294967296;
    bars.push(0.12 + (state / 4294967296) * 0.88);
  }
  return bars;
}

function hashString(value: string): number {
  let hash = 2166136261;
  for (const character of value) {
    hash = (hash ^ (character.codePointAt(0) ?? 0)) * 16777619;
    hash %= 4294967296;
  }
  return Math.abs(hash);
}

/** Fraction of the waveform that has been played, 0..1. */
export function waveformProgress(positionSeconds: number, durationSeconds: number): number {
  if (!Number.isFinite(durationSeconds) || durationSeconds <= 0) return 0;
  return Math.min(1, Math.max(0, positionSeconds / durationSeconds));
}

/* ===== Segmented / Tabs shared helper ===== */

/** Roving-focus index maths: wraps around, never leaves the list. */
export function nextIndexOf(
  current: number,
  delta: number,
  length: number,
  options: { wrap?: boolean } = {},
): number {
  if (length <= 0) return -1;
  const raw = current + delta;
  if (options.wrap === false) return Math.min(length - 1, Math.max(0, raw));
  return ((raw % length) + length) % length;
}

/** Typeahead target: index of the first entry whose label starts with the query. */
export function typeaheadIndex(labels: readonly string[], query: string, from = 0): number {
  const needle = query.trim().toLowerCase();
  if (needle.length === 0 || labels.length === 0) return -1;
  for (let offset = 0; offset < labels.length; offset += 1) {
    const index = (from + offset) % labels.length;
    if ((labels[index] ?? '').trim().toLowerCase().startsWith(needle)) return index;
  }
  return -1;
}
