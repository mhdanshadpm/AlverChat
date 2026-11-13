import { useCallback, useEffect, useState, type ReactNode } from 'react';
import { ToastViewport, type ToastView } from '../components';
import type { Peer } from '../../../bridge/src';
import { ScreenHeader, useAppBridge, type ScreenProps, type ToastTone, type View } from './shared';

/**
 * Mounts one screen on its own at `#/preview/<screen>` with the mock bridge, so
 * a screen can be designed, screenshotted and reviewed without the full shell.
 */
export function PreviewHost({ children }: { children: (props: ScreenProps) => ReactNode }) {
  const { bridge, settings, setSettings, identity } = useAppBridge();
  const [peers, setPeers] = useState<Peer[]>([]);
  const [view, setView] = useState<View>('settings');
  const [activeChat, setActiveChat] = useState<string | null>(null);
  const [toasts, setToasts] = useState<readonly ToastView[]>([]);
  const [dark, setDark] = useState(settings.theme === 'dark');

  useEffect(() => setDark(settings.theme === 'dark' || (settings.theme === 'system' && globalThis.window?.matchMedia('(prefers-color-scheme: dark)').matches)), [settings.theme]);
  useEffect(() => { void bridge.listPeers().then(setPeers); }, [bridge]);

  const toast = useCallback((message: string, tone: ToastTone = 'neutral') => {
    setToasts(current => [...current.slice(-2), { id: `${Date.now()}-${Math.random()}`, message, tone }]);
  }, []);

  const go = useCallback((next: View, chatId?: string) => { setView(next); if (chatId) setActiveChat(chatId); }, []);

  const patchSettings = useCallback((patch: Partial<typeof settings>) => {
    setSettings(current => ({ ...current, ...patch }));
    void bridge.updateSettings(patch).then(setSettings).catch(error => toast(String(error), 'danger'));
  }, [bridge, setSettings, toast]);

  const props: ScreenProps = { bridge, view, go, toast, peers, activeChat, settings, patchSettings, identity };

  return (
    <main className={dark ? 'app dark' : 'app'} style={{ height: '100vh' }}>
      <div className="main-area preview-area">
        <div className="preview-scroll">
          <ScreenHeader eyebrow="Screen preview" title={view} description="Rendered through the mock bridge. Real data replaces this at runtime." />
          {children(props)}
        </div>
      </div>
      <ToastViewport toasts={toasts} onDismiss={id => setToasts(current => current.filter(toast => toast.id !== id))} />
    </main>
  );
}
