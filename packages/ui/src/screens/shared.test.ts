import { describe, expect, it } from 'vitest';
import type { Peer } from '../../../bridge/src';
import { authorName } from './shared';

const ANU = '3b6ae924015cc74cd7f33ff4e3cf89d64a9472ca5f0631acdae89f3795324738';
const MINE = 'a1cd2f0e4b6789a0c1d2e3f405162738495a6b7c8d9e0f112233445566778899';

const PEERS: Peer[] = [
  { id: ANU, name: 'Anu Hasaan' } as Peer,
  { id: MINE, name: 'Bala' } as Peer,
];

describe('authorName', () => {
  it('reads a discovered peer by the name this device knows it as', () => {
    expect(authorName(ANU, PEERS, { id: MINE, name: 'Bala' })).toBe('Anu Hasaan');
  });

  it('answers this device\'s own id with the display name it advertises', () => {
    // Storage writes `local` for a message this device sent, whichever adapter wrote it.
    expect(authorName(MINE, PEERS, { id: MINE, name: 'Bala' })).toBe('Bala');
    expect(authorName('local', PEERS, { id: MINE, name: 'Bala' })).toBe('Bala');
    expect(authorName('me', PEERS, { id: MINE, name: 'Bala' })).toBe('Bala');
  });

  it('shortens an id nobody can name rather than printing sixty-four characters', () => {
    const stranger = 'ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff';
    expect(authorName(stranger, [], { id: MINE, name: 'Bala' })).toBe('ffffffff…');
  });

  it('leaves anything that is already a name alone', () => {
    expect(authorName('Arjun Kumar', PEERS)).toBe('Arjun Kumar');
    expect(authorName('group-design', PEERS)).toBe('group-design');
  });

  it('falls back to the peer list when there is no self to answer for', () => {
    expect(authorName(ANU, PEERS)).toBe('Anu Hasaan');
  });
});
