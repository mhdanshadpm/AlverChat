import { describe, expect, it } from 'vitest';
import type { ChatMessage, Group, Peer } from '../../../bridge/src';
import { groupPeer, rosterRows, summaryOf } from '../groupChats';

const GROUP: Group = {
  id: 'group-billing',
  name: 'Billing Team',
  epoch: 3,
  admin: 'me',
  members: ['peer-anu', 'peer-sarath', 'me'],
};

function message(partial: Partial<ChatMessage> & { id: string }): ChatMessage {
  return { sender: 'peer-anu', text: '', time: '', ...partial };
}

describe('groupPeer', () => {
  it('synthesizes a chat-list row for a group thread', () => {
    const peer = groupPeer(GROUP, { preview: 'Purchase entry completed.', time: '10:42', unread: 2 });
    expect(peer).toMatchObject({
      id: 'group-billing',
      name: 'Billing Team',
      initials: 'BT',
      status: '3 members',
      preview: 'Purchase entry completed.',
      time: '10:42',
      unread: 2,
      group: true,
    });
  });

  it('omits the unread badge when nothing is unread', () => {
    expect(groupPeer(GROUP, { preview: '', time: '', unread: 0 }).unread).toBeUndefined();
    expect(groupPeer(GROUP, undefined).unread).toBeUndefined();
  });

  it('reads the palette out of an emoji icon and defaults to violet', () => {
    expect(groupPeer({ ...GROUP, icon: '🚀 blue' }, undefined).color).toBe('blue');
    expect(groupPeer(GROUP, undefined).color).toBe('violet');
  });

  it('borrows the typing flag from its roster, since a group has no session', () => {
    const rows = [
      { id: 'peer-anu', typing: true },
      { id: 'peer-sarath', typing: false },
    ] as Peer[];
    expect(groupPeer(GROUP, undefined, rows).typing).toBe(true);
    expect(groupPeer(GROUP, undefined, [rows[1]]).typing).toBe(false);
    expect(groupPeer(GROUP, undefined).typing).toBe(false);
  });

  it('lists roster rows in roster order and skips peers nobody has discovered', () => {
    const anu = { id: 'peer-anu' } as Peer;
    expect(rosterRows(GROUP, new Map([['peer-anu', anu]])).map(row => row.id)).toEqual(['peer-anu']);
    expect(rosterRows(GROUP, new Map())).toEqual([]);
  });
});

describe('summaryOf', () => {
  it('takes the last message as the preview and counts unread incoming ones', () => {
    const summary = summaryOf([
      message({ id: '1', text: 'first', time: '10:00', mine: false, read: true }),
      message({ id: '2', text: 'second', time: '10:05', mine: false, read: false }),
      message({ id: '3', text: 'mine', time: '10:06', mine: true, read: true }),
    ]);
    expect(summary.preview).toBe('mine');
    expect(summary.time).toBe('10:06');
    expect(summary.unread).toBe(1);
  });

  it('shows the file name for a file message', () => {
    const summary = summaryOf([
      message({ id: '1', kind: 'file', fileName: 'Invoice_ML00037.pdf', time: '12:00', mine: false, read: false }),
    ]);
    expect(summary.preview).toBe('Invoice_ML00037.pdf');
    expect(summary.unread).toBe(1);
  });

  it('returns an empty summary for a silent thread', () => {
    expect(summaryOf([])).toEqual({ preview: '', time: '', unread: 0 });
  });

  it('ignores event rows when counting unread', () => {
    const summary = summaryOf([
      message({ id: '1', kind: 'event', text: 'Arjun changed the group icon', time: '9:00', mine: false, read: false }),
    ]);
    expect(summary.preview).toBe('Arjun changed the group icon');
    expect(summary.unread).toBe(0);
  });
});
