import { useEffect, useState, type ReactNode } from 'react';
import type { AppSettings, ChatMessage, Peer, PlatformBridge } from '../../../bridge/src';
import { DEFAULT_SETTINGS, bridgeForRuntime, emptyBrowserBridge } from '../../../bridge/src';

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
  identity: { peerId: string; fingerprint: string } | null;
};

/** Opens on the profile step when the display name has never been set. */
export function needsOnboarding(settings: AppSettings): boolean {
  return settings.displayName.trim().length === 0;
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
      <h2>{title}</h2>
      {description ? <p>{description}</p> : null}
      {actions ? <div className="screen-header-actions">{actions}</div> : null}
    </div>
  );
}

/** Bridge + settings + identity, resolved once per window. */
export function useAppBridge() {
  const [bridge, setBridge] = useState<PlatformBridge>(emptyBrowserBridge);
  const [settings, setSettings] = useState<AppSettings>(DEFAULT_SETTINGS);
  const [identity, setIdentity] = useState<{ peerId: string; fingerprint: string } | null>(null);
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
  return { bridge, settings, setSettings, identity, ready };
}
