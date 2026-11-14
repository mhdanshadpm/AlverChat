import { initialsFor } from './components';
import { messageTime } from './screens/shared';
import type { ChatMessage, Group, Peer } from '../../bridge/src';

/** Latest-message summary of a group thread, derived from the persisted messages. */
export type GroupThread = { preview: string; time: string; unread: number };

/**
 * Groups are not discovery peers, so a chat list row is synthesized from the group
 * roster and its thread. `status` doubles as the member count line.
 */
export function groupPeer(group: Group, thread: GroupThread | undefined): Peer {
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
  };
}

export function summaryOf(messages: readonly ChatMessage[]): GroupThread {
  const last = messages[messages.length - 1];
  return {
    preview: last ? (last.kind === 'file' ? (last.fileName ?? 'Attachment') : last.text) : '',
    time: last ? messageTime(last) : '',
    unread: messages.filter(message => !message.mine && message.kind !== 'event' && !message.read).length,
  };
}
