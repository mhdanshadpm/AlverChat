import { useEffect, useSyncExternalStore } from 'react';
import { Avatar, type AvatarProps } from './Avatar';
import {
  loadPeerAvatar,
  peerAvatarPhoto,
  peerAvatarVersion,
  subscribePeerAvatars,
  type PeerAvatarSource,
} from '../peerAvatars';

export type PeerAvatarProps = Omit<AvatarProps, 'src'> & {
  /** The bridge is the only thing that can pull a peer's photo. */
  source: PeerAvatarSource;
  /** Whose photo to draw — a group, `me` or an unknown id stays on initials. */
  peerId?: string | null;
};

/**
 * An Avatar that draws the photo a peer publishes over the LAN. The bytes come from a
 * shared cache, so every surface naming the same peer shows the same picture and asks
 * for it once; until it arrives — or if the peer publishes none — the initials variant
 * the design boards show is what renders.
 */
export function PeerAvatar({ source, peerId, ...props }: PeerAvatarProps) {
  const version = useSyncExternalStore(subscribePeerAvatars, peerAvatarVersion, peerAvatarVersion);
  useEffect(() => {
    if (peerId) loadPeerAvatar(source, peerId);
  }, [peerId, source, version]);
  return <Avatar {...props} src={peerAvatarPhoto(peerId) ?? undefined} />;
}
