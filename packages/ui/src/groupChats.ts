import { initialsFor } from './components';
import { messageTime } from './screens/shared';
import type { ChatMessage, Group, Peer } from '../../bridge/src';

/** Latest-message summary of a group thread, derived from the persisted messages. */
export type GroupThread = { preview: string; time: string; unread: number };

/**
 * Groups are not discovery peers, so a chat list row is synthesized from the group
 * roster and its thread. `status` doubles as the member count line.
 *
 * A group has no session of its own, so `members` — the live peer rows of the roster —
 * decide its typing flag: the group is typing when anybody in it is.
 */
export function groupPeer(
  group: Group,
  thread: GroupThread | undefined,
  members: readonly Peer[] = [],
): Peer {
  const palette = group.icon?.match(/peach|rose|blue|green|slate|violet/)?.[0] ?? 'violet';
  return {
    id: group.id,
    name: group.name,
    initials: initialsFor(group.name),
    color: palette,
    online: true,
    status: `${group.members.length} members`,
    preview: thread?.preview ?? '',
    time: thread?.time ?? '',
    unread: thread && thread.unread > 0 ? thread.unread : undefined,
    group: true,
    typing: members.some(member => member.typing),
  };
}

/** The live peer rows of a group's roster, in roster order. */
export function rosterRows(group: Group, byId: ReadonlyMap<string, Peer>): Peer[] {
  const rows: Peer[] = [];
  for (const id of group.members) {
    const row = byId.get(id);
    if (row) rows.push(row);
  }
  return rows;
}

export function summaryOf(messages: readonly ChatMessage[]): GroupThread {
  const last = messages[messages.length - 1];
  return {
    preview: last ? (last.kind === 'file' ? (last.fileName ?? 'Attachment') : last.text) : '',
    time: last ? messageTime(last) : '',
    unread: messages.filter(message => !message.mine && message.kind !== 'event' && !message.read).length,
  };
}
