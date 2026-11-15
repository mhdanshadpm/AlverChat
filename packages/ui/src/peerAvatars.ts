/* ------------------------------------------------------------------ *
 * Peer photos, cached once for the whole shell.
 *
 * A peer avatar is not in the peer list: it is a picture the other machine
 * agreed to publish, pulled over the LAN only after a heartbeat says it
 * changed. So the cache is keyed by peer and asked exactly once per peer —
 * the chat header, the People row and the details panel must not each start
 * their own fetch for the same bytes, and they must agree on what they show.
 * ------------------------------------------------------------------ */

import { avatarFromDataUrl } from './screens/photoPipeline';

/** The part of the bridge this cache reads; keeps the module testable without Tauri. */
export type PeerAvatarSource = { getPeerAvatar(peerId: string): Promise<string | null> };

/** `undefined` = never asked, `null` = asked and that peer publishes no photo. */
const photos = new Map<string, string | null>();
const asked = new Set<string>();
let version = 0;
const listeners = new Set<() => void>();

function notify(): void {
  version += 1;
  for (const listener of listeners) listener();
}

export function subscribePeerAvatars(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** Stable between changes, so React can use it as an external-store snapshot. */
export function peerAvatarVersion(): number {
  return version;
}

export function peerAvatarPhoto(peerId: string | undefined | null): string | null | undefined {
  if (!peerId) return null;
  return photos.get(peerId);
}

/**
 * Reads one peer photo, once. A second call for the same peer is free, and a read that
 * fails is made retryable again — a peer that answers later still gets its picture.
 */
export function loadPeerAvatar(source: PeerAvatarSource, peerId: string): void {
  if (!peerId || asked.has(peerId)) return;
  asked.add(peerId);
  source.getPeerAvatar(peerId).then(
    // Validated on the way in, so a corrupt or oversized photo falls back to initials
    // instead of ever reaching an <img src>.
    (dataUrl) => {
      photos.set(peerId, avatarFromDataUrl(dataUrl));
      notify();
    },
    () => {
      asked.delete(peerId);
    },
  );
}

/**
 * A heartbeat said the photos moved. Forget what we hold — for one peer, or for every
 * peer — so the surfaces still on screen ask again and repaint.
 */
export function invalidatePeerAvatars(peerId?: string): void {
  if (peerId === undefined) {
    photos.clear();
    asked.clear();
  } else {
    photos.delete(peerId);
    asked.delete(peerId);
  }
  notify();
}
