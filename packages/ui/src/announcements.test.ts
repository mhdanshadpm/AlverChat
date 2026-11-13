import { describe, expect, it } from 'vitest';
import type { Announcement, PlatformBridge } from '../../bridge/src';
import {
  announcementView,
  canPostAnnouncement,
  hasAcknowledged,
  isSelfId,
  loadGroupAnnouncements,
  loadPendingAnnouncements,
  newestFirst,
  needsAcknowledgement,
} from './announcements';

const at = (
  over: Partial<Announcement> & Pick<Announcement, 'id' | 'groupId' | 'author' | 'text'>,
): Announcement => ({
  acknowledgedBy: [],
  authorName: 'Peer',
  createdMs: 1_000,
  ...over,
});

const mine = (over: Partial<Announcement> = {}) =>
  at({ author: 'me', authorName: 'You', groupId: 'g1', id: 'mine', text: 'My own post', ...over });

describe('announcement authority', () => {
  it('lets anyone post until the admins-only setting is on', () => {
    expect(canPostAnnouncement(false, false)).toBe(true);
    expect(canPostAnnouncement(true, false)).toBe(true);
    expect(canPostAnnouncement(false, true)).toBe(false);
    expect(canPostAnnouncement(true, true)).toBe(true);
  });
});

describe('announcement identity', () => {
  it('knows this device whatever the adapter called it', () => {
    expect(isSelfId('me', 'peer-7')).toBe(true);
    expect(isSelfId('local', 'peer-7')).toBe(true);
    expect(isSelfId('peer-7', 'peer-7')).toBe(true);
    expect(isSelfId('peer-8', 'peer-7')).toBe(false);
    expect(isSelfId('peer-7', null)).toBe(false);
  });

  it('never asks the author to acknowledge their own post', () => {
    expect(needsAcknowledgement(mine(), 'peer-7')).toBe(false);
    const theirs = at({ author: 'peer-8', groupId: 'g1', id: 'a2', text: 'Maintenance' });
    expect(needsAcknowledgement(theirs, 'peer-7')).toBe(true);
    expect(hasAcknowledged({ ...theirs, acknowledgedBy: ['peer-7'] }, 'peer-7')).toBe(true);
    expect(needsAcknowledgement({ ...theirs, acknowledgedBy: ['peer-9', 'me'] }, 'peer-7')).toBe(false);
  });
});

describe('announcement ordering and views', () => {
  const older = at({ author: 'peer-8', createdMs: 10, groupId: 'g1', id: 'old', text: 'Older' });
  const newer = at({ author: 'peer-8', createdMs: 20, groupId: 'g2', id: 'new', text: 'Newer' });

  it('sorts newest first without touching the input', () => {
    const input = [older, newer];
    expect(newestFirst(input).map((item) => item.id)).toEqual(['new', 'old']);
    expect(input.map((item) => item.id)).toEqual(['old', 'new']);
  });

  it('breaks a tie deterministically by id', () => {
    const tied = [
      at({ author: 'peer-8', createdMs: 20, groupId: 'g1', id: 'aa', text: 'A' }),
      at({ author: 'peer-8', createdMs: 20, groupId: 'g1', id: 'bb', text: 'B' }),
    ];
    expect(newestFirst(tied).map((item) => item.id)).toEqual(['bb', 'aa']);
  });

  it('turns ids into the names the banner shows', () => {
    const names: Record<string, string> = { 'peer-8': 'Arjun', 'peer-9': 'Fathima' };
    const view = announcementView(
      at({ acknowledgedBy: ['peer-9'], author: 'peer-8', groupId: 'g1', id: 'a1', text: 'Maintenance' }),
      (id) => names[id] ?? id,
      'peer-7',
      'Everyone',
    );
    expect(view).toMatchObject({
      acknowledgedBy: ['Fathima'],
      from: 'Arjun',
      group: 'Everyone',
      id: 'a1',
      isMine: false,
      text: 'Maintenance',
    });
    expect(view.at).toBeTruthy();
  });

  it('labels my own post with the name the bridge stored for me', () => {
    const view = announcementView(mine(), (id) => id, 'peer-7');
    expect(view.from).toBe('You');
    expect(view.isMine).toBe(true);
  });
});

describe('announcement loading', () => {
  it('treats an unreadable group as empty instead of throwing', async () => {
    const failing = {
      listAnnouncements: () => Promise.reject(new Error('locked database')),
    } as unknown as PlatformBridge;
    expect(await loadGroupAnnouncements(failing, 'g1')).toEqual([]);
  });

  it('collects across groups and keeps only what still needs a tap', async () => {
    const seen = at({
      acknowledgedBy: ['peer-7'],
      author: 'peer-8',
      createdMs: 50,
      groupId: 'g1',
      id: 'seen',
      text: 'Already acknowledged',
    });
    const fresh = at({ author: 'peer-8', createdMs: 40, groupId: 'g2', id: 'fresh', text: 'Fresh' });
    const fake = {
      listGroups: () => Promise.resolve([{ id: 'g1' }, { id: 'g2' }] as never),
      listAnnouncements: (groupId: string) =>
        Promise.resolve(groupId === 'g1' ? [seen] : [mine(), fresh]),
    } as unknown as PlatformBridge;
    expect((await loadPendingAnnouncements(fake, 'peer-7')).map((item) => item.id)).toEqual(['fresh']);
  });

  it('survives a bridge that cannot list groups', async () => {
    const fake = { listGroups: () => Promise.reject(new Error('no store')) } as unknown as PlatformBridge;
    expect(await loadPendingAnnouncements(fake, 'peer-7')).toEqual([]);
  });
});
