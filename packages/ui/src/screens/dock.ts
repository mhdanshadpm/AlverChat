import { APP_NAME } from '../../../bridge/src';
import { ns, registerNamespace } from '../i18n';
import shellCatalog from '../i18n/en/shell';

registerNamespace('shell', shellCatalog);
const text = ns('shell');

/** Settings → Opacity is a 20–100 percentage. */
export function dockOpacity(opacityPercent: number): number {
  if (!Number.isFinite(opacityPercent)) return 1;
  // Out-of-range values clamp instead of passing through: a dock the user cannot see
  // at all is worse than a slightly transparent one, and `opacity: 0` would leave an
  // invisible hit target that still swallows clicks.
  return Math.min(1, Math.max(0.2, opacityPercent / 100));
}

/**
 * The photo the idle icon shows, or `null` for the logo. "My Profile Photo" only takes
 * over while nothing is unread — the unread avatar is the point of the icon — and never
 * replaces it with an empty circle when no photo has been set.
 */
export function idlePhotoFor(
  idleIcon: 'logo' | 'photo',
  unreadTotal: number,
  photo: string | null,
): string | null {
  if (idleIcon !== 'photo' || unreadTotal > 0) return null;
  return photo && photo.length > 0 ? photo : null;
}

export type DockIconMood = 'call' | 'dnd' | 'default';

/**
 * Which ring the icon wears. A live call outranks a quiet device: the call is what the
 * user is in the middle of, and the board draws it as its own frame.
 */
export function dockIconMood(inCall: boolean, quiet: boolean): DockIconMood {
  if (inCall) return 'call';
  return quiet ? 'dnd' : 'default';
}

/** The board counts a call up as `MM:SS`; 99:59 caps it so the chip cannot outgrow the icon. */
export function callClockLabel(startedMs: number, nowMs: number): string {
  if (!Number.isFinite(startedMs) || !Number.isFinite(nowMs)) return '00:00';
  const total = Math.min(5_999, Math.max(0, Math.floor((nowMs - startedMs) / 1_000)));
  const minutes = Math.floor(total / 60);
  const seconds = total % 60;
  return `${String(minutes).padStart(2, '0')}:${String(seconds).padStart(2, '0')}`;
}

/**
 * Chats arrive in whatever order the shell keeps them (alphabetically on desktop), so
 * recency has to be applied here: an unread from an hour ago must not crowd out one that
 * landed a second ago. Chats with no stamp sort first, i.e. least recent.
 */
export function oldestFirst<T extends { lastMessageMs?: number; name: string }>(items: T[]): T[] {
  return [...items].sort(
    (a, b) => (a.lastMessageMs ?? 0) - (b.lastMessageMs ?? 0) || a.name.localeCompare(b.name),
  );
}

/**
 * The timestamp a row or bubble shows. The desktop shell stamps `lastMessageMs` and
 * leaves `time` empty; the browser preview carries only the pre-formatted `time`.
 */
export function peerTimeLabel(peer: { time?: string; lastMessageMs?: number }): string {
  if (peer.lastMessageMs && Number.isFinite(peer.lastMessageMs)) {
    return new Date(peer.lastMessageMs).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });
  }
  return peer.time ?? '';
}

/**
 * Whether the icon wears the active chat's face. "Icon morph (on activity)" is the whole
 * feature: with it off the dock keeps its idle logo or profile photo and only the badge
 * counts. With it on, the face hands the icon back after "Revert delay" so the dock is
 * not left wearing a chat nobody is looking at — except during a call, which is not a
 * passing event.
 */
export function showMorphedIcon(enabled: boolean, reverted: boolean, inCall: boolean): boolean {
  if (!enabled) return false;
  return inCall || !reverted;
}

/**
 * The revert timer in milliseconds. The slider offers 1–30 s, and a restored backup or a
 * hand-edited settings file can hold anything in a `u8`, so the range is re-applied here
 * rather than scheduling a five-minute stare or a timer that fires before the paint.
 */
export function morphRevertMs(seconds: number): number {
  const clamped = Number.isFinite(seconds) ? Math.min(30, Math.max(1, seconds)) : 5;
  return clamped * 1000;
}

/**
 * The board's hover card: who the icon is about, and what it is counting. A native
 * tooltip is used because the dock is a borderless always-on-top window, where a
 * drawn card would need its own resize and hit-region work.
 */
export function dockTooltip(options: { inCall: boolean; peerName: string; unreadTotal: number; quiet: boolean }): string {
  const name = options.peerName.trim();
  if (options.inCall) return name ? text('dockInCallWith', 'In call with {{name}}', { name }) : text('dockInCall', 'In a call');
  if (options.unreadTotal > 0) {
    const one = options.unreadTotal === 1;
    if (name) {
      return one
        ? text('dockNewMessagesForOne', '{{name}} · {{count}} new message', { name, count: options.unreadTotal })
        : text('dockNewMessagesFor', '{{name}} · {{count}} new messages', { name, count: options.unreadTotal });
    }
    return one
      ? text('dockNewMessagesOne', '{{count}} new message', { count: options.unreadTotal })
      : text('dockNewMessages', '{{count}} new messages', { count: options.unreadTotal });
  }
  return options.quiet ? text('dockNotificationsPaused', 'Notifications are paused') : APP_NAME;
}
