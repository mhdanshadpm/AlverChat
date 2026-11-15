import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  invalidatePeerAvatars,
  loadPeerAvatar,
  peerAvatarPhoto,
  peerAvatarVersion,
  subscribePeerAvatars,
  type PeerAvatarSource,
} from './peerAvatars';

const PNG = 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==';

const flush = () => new Promise<void>(resolve => setTimeout(resolve, 0));

/** A bridge whose peer photos are scripted per peer, counting how often each is asked. */
function scripted(answers: Record<string, string | null | Error>): PeerAvatarSource & { asks: Record<string, number> } {
  const asks: Record<string, number> = {};
  return {
    asks,
    getPeerAvatar(peerId) {
      asks[peerId] = (asks[peerId] ?? 0) + 1;
      const answer = answers[peerId];
      return answer instanceof Error ? Promise.reject(answer) : Promise.resolve(answer ?? null);
    },
  };
}

beforeEach(() => {
  invalidatePeerAvatars();
});

describe('peer avatar cache', () => {
  it('asks a peer once no matter how many surfaces want the photo', async () => {
    const source = scripted({ 'peer-arjun': PNG });
    loadPeerAvatar(source, 'peer-arjun');
    loadPeerAvatar(source, 'peer-arjun');
    loadPeerAvatar(source, 'peer-arjun');
    await flush();
    expect(source.asks['peer-arjun']).toBe(1);
    expect(peerAvatarPhoto('peer-arjun')).toBe(PNG);
  });

  it('remembers a peer that publishes no photo instead of asking again', async () => {
    const source = scripted({ 'peer-vishnu': null });
    loadPeerAvatar(source, 'peer-vishnu');
    await flush();
    expect(peerAvatarPhoto('peer-vishnu')).toBeNull();
    loadPeerAvatar(source, 'peer-vishnu');
    await flush();
    expect(source.asks['peer-vishnu']).toBe(1);
  });

  it('lets a peer that failed to answer be asked again', async () => {
    const source = scripted({ 'peer-anu': new Error('session closed') });
    loadPeerAvatar(source, 'peer-anu');
    await flush();
    expect(peerAvatarPhoto('peer-anu')).toBeUndefined();
    loadPeerAvatar(source, 'peer-anu');
    await flush();
    expect(source.asks['peer-anu']).toBe(2);
  });

  it('refuses a value that is not a safe image data URL', async () => {
    const source = scripted({
      'peer-deepa': 'https://host.example/photo.jpg',
      'peer-fathima': 'data:image/svg+xml;base64,PHN2Zz48L3N2Zz4=',
    });
    loadPeerAvatar(source, 'peer-deepa');
    loadPeerAvatar(source, 'peer-fathima');
    await flush();
    expect(peerAvatarPhoto('peer-deepa')).toBeNull();
    expect(peerAvatarPhoto('peer-fathima')).toBeNull();
  });

  it('forgets one peer when a heartbeat says their photo moved', async () => {
    const source = scripted({ 'peer-arjun': PNG });
    loadPeerAvatar(source, 'peer-arjun');
    await flush();
    invalidatePeerAvatars('peer-arjun');
    expect(peerAvatarPhoto('peer-arjun')).toBeUndefined();
    loadPeerAvatar(source, 'peer-arjun');
    await flush();
    expect(source.asks['peer-arjun']).toBe(2);
    expect(peerAvatarPhoto('peer-arjun')).toBe(PNG);
  });

  it('tells every subscriber, and only once, per change', async () => {
    const heard = vi.fn();
    const stop = subscribePeerAvatars(heard);
    const before = peerAvatarVersion();
    loadPeerAvatar(scripted({ 'peer-arjun': PNG }), 'peer-arjun');
    await flush();
    stop();
    loadPeerAvatar(scripted({ 'peer-sarath': PNG }), 'peer-sarath');
    await flush();
    expect(heard).toHaveBeenCalledTimes(1);
    expect(peerAvatarVersion()).toBe(before + 2);
  });

  it('names an unknown peer as nothing to draw, so initials stay up', () => {
    expect(peerAvatarPhoto(undefined)).toBeNull();
    expect(peerAvatarPhoto('')).toBeNull();
    expect(peerAvatarPhoto('peer-never-seen')).toBeUndefined();
  });
});
