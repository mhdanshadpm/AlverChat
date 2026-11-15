import { useEffect, useState, type ReactNode } from 'react';
import type { AppSettings, ChatMessage, Identity, Peer, PlatformBridge } from '../../../bridge/src';
import { DEFAULT_SETTINGS, bridgeForRuntime, emptyBrowserBridge } from '../../../bridge/src';
import { isSelfId } from '../announcements';
import { setLanguage } from '../i18n';

export type View = 'chats' | 'groups' | 'people' | 'calls' | 'settings' | 'profile' | 'onboarding';

export type ToastTone = 'neutral' | 'accent' | 'success' | 'danger';

export type ScreenProps = {
  /** The resolved bridge: Tauri in the desktop app, the mock under ?mock=1. */
  bridge: PlatformBridge;
  view: View;
  go: (view: View, chatId?: string) => void;
  toast: (message: string, tone?: ToastTone) => void;
  peers: Peer[];
  activeChat: string | null;
  settings: AppSettings;
  /** Optimistic patch; the shell owns persistence through the bridge. */
  patchSettings: (patch: Partial<AppSettings>) => void;
  /** Call after announcement storage changes, so the shell re-reads its banner. */
  refreshAnnouncements?: () => void;
  /** Bumped whenever stored traffic changes, so screens caching a list can re-read it. */
  refreshKey?: number;
  identity: Identity | null;
  /** Starts a 1-to-1 call; the shell owns media and signalling. */
  onCall?: (chatId: string, media: 'audio' | 'video') => void;
};

/** Opens on the profile step when the display name has never been set. */
export function needsOnboarding(settings: AppSettings): boolean {
  return settings.displayName.trim().length === 0;
}

/** A peer id is a long lowercase hex string; a shorter or non-hex value is already a name. */
const PEER_ID = /^[0-9a-f]{32,}$/;

/**
 * The name to print for a sender, a reactor or a group member.
 *
 * Storage keeps people as peer ids, so a row read straight from it can name someone as
 * `3b6ae924015cc74c…`, and an id printed in a bubble is not a name. A peer this device has
 * discovered is read from the list, this device's own rows take the display name it
 * advertises, and an id nobody can name is shortened so a thread still tells two strangers
 * apart. Anything not shaped like an id is handed back unchanged, which is what the mock
 * bridge and the component gallery already pass in.
 */
export function authorName(
  id: string,
  peers: readonly Peer[],
  self?: { id: string; name: string },
): string {
  if (self && isSelfId(id, self.id)) return self.name;
  const known = peers.find((peer) => peer.id === id);
  if (known?.name) return known.name;
  return PEER_ID.test(id) ? `${id.slice(0, 8)}…` : id;
}

export function messageTime(message: ChatMessage): string {
  if (message.time) return message.time;
  if (message.timestampMs) return new Date(message.timestampMs).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });
  return '';
}

export function ScreenHeader({ eyebrow, title, description, actions }: { eyebrow: string; title: string; description?: string; actions?: ReactNode }) {
  return (
    <div className="screen-header">
      <span>{eyebrow}</span>
      <h1>{title}</h1>
      {description ? <p>{description}</p> : null}
      {actions ? <div className="screen-header-actions">{actions}</div> : null}
    </div>
  );
}

/** Bridge + settings + identity, resolved once per window. */
export function useAppBridge() {
  const [bridge, setBridge] = useState<PlatformBridge>(emptyBrowserBridge);
  const [settings, setSettings] = useState<AppSettings>(DEFAULT_SETTINGS);
  const [identity, setIdentity] = useState<Identity | null>(null);
  /** False until the bridge has answered, so the shell can avoid flashing the onboarding gate. */
  const [ready, setReady] = useState(false);
  useEffect(() => {
    let mounted = true;
    void bridgeForRuntime().then(async next => {
      if (!mounted) return;
      setBridge(next);
      setSettings(await next.getSettings());
      setIdentity(await next.getMyIdentity());
      if (mounted) setReady(true);
    });
    return () => { mounted = false; };
  }, []);
  /* Settings' language is the only store for the language: here is the half that reads it.
     Without this the picker wrote a value nothing asked i18next for, and every screen kept
     answering keys in English whatever it said. */
  useEffect(() => {
    setLanguage(settings.language);
  }, [settings.language]);
  return { bridge, settings, setSettings, identity, ready };
}
