import type { Announcement, PlatformBridge } from '../../bridge/src';

/**
 * LAN announcements (spec 7.12). Storage speaks peer ids and epoch stamps; the
 * banner speaks display names. Everything that converts one to the other lives
 * here so the screens stay declarative.
 */

/** Resolves a peer id to the name the local device knows for it. */
export type NameLookup = (id: string) => string;

/** Ids the bridge uses for this device: the mock, the store, and the real peer id. */
const SELF_IDS = new Set(['me', 'self', 'local']);

/** True when `id` names this device, whichever adapter produced the row. */
export function isSelfId(id: string, peerId?: string | null): boolean {
  return SELF_IDS.has(id) || (Boolean(peerId) && id === peerId);
}

/** Newest first, which is both the panel order and the banner order. */
export function newestFirst(items: readonly Announcement[]): Announcement[] {
  return [...items].sort((a, b) => b.createdMs - a.createdMs || (a.id < b.id ? 1 : -1));
}

/** Whether this device already tapped Acknowledge. */
export function hasAcknowledged(announcement: Announcement, peerId?: string | null): boolean {
  return announcement.acknowledgedBy.some((id) => isSelfId(id, peerId));
}

/** The author never acknowledges their own post, so only other people's rows ask. */
export function needsAcknowledgement(announcement: Announcement, peerId?: string | null): boolean {
  return !isSelfId(announcement.author, peerId) && !hasAcknowledged(announcement, peerId);
}

/** Who may post: anyone by default, admins only when the setting says so. */
export function canPostAnnouncement(isAdmin: boolean, adminsOnly: boolean): boolean {
  return !adminsOnly || isAdmin;
}

/** Date + time, since an announcement stays in view for hours. */
export function announcementTime(createdMs: number): string {
  return new Date(createdMs).toLocaleString([], {
    day: 'numeric',
    hour: 'numeric',
    minute: '2-digit',
    month: 'short',
  });
}

/** The shape `AnnouncementBanner` draws: names, not ids. */
export type AnnouncementView = {
  at: string;
  acknowledgedBy: string[];
  from: string;
  group?: string;
  id: string;
  isMine: boolean;
  text: string;
};

export function announcementView(
  announcement: Announcement,
  nameOf: NameLookup,
  peerId?: string | null,
  group?: string,
): AnnouncementView {
  const mine = isSelfId(announcement.author, peerId);
  return {
    acknowledgedBy: announcement.acknowledgedBy.map(nameOf),
    at: announcementTime(announcement.createdMs),
    from: mine && announcement.authorName ? announcement.authorName : nameOf(announcement.author),
    group,
    id: announcement.id,
    isMine: mine,
    text: announcement.text,
  };
}

/** The announcements of one group, newest first; a failed read is an empty list. */
export async function loadGroupAnnouncements(
  bridge: PlatformBridge,
  groupId: string,
): Promise<Announcement[]> {
  try {
    return newestFirst(await bridge.listAnnouncements(groupId));
  } catch {
    return [];
  }
}

/**
 * Rows still owed a tap, across the groups this device belongs to.
 *
 * The bridge has no "all announcements" call, so the group list is read once and
 * each group is asked. A group that fails to answer is skipped rather than
 * hiding the banner for the rest.
 */
export async function loadPendingAnnouncements(
  bridge: PlatformBridge,
  peerId?: string | null,
): Promise<Announcement[]> {
  let groupIds: string[];
  try {
    groupIds = (await bridge.listGroups()).map((group) => group.id);
  } catch {
    return [];
  }
  const pages = await Promise.all(groupIds.map((id) => loadGroupAnnouncements(bridge, id)));
  return newestFirst(pages.flat().filter((item) => needsAcknowledgement(item, peerId)));
}
