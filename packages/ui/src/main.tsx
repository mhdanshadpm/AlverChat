import React, { useEffect, useState, type ReactNode } from 'react';
import '@fontsource/inter/400.css';
import '@fontsource/inter/500.css';
import '@fontsource/inter/600.css';
import '@fontsource/inter/700.css';
import '@fontsource/manrope/500.css';
import '@fontsource/manrope/600.css';
import '@fontsource/manrope/700.css';
import '@fontsource/manrope/800.css';
import { createRoot } from 'react-dom/client';
import App from './screens/App';
import { CompactWindow, DockWindow, MenuWindow } from './screens/OverlayWindows';
import { Gallery } from './screens/Gallery';
import { PreviewHost } from './screens/PreviewHost';
import type { ScreenProps } from './screens/shared';
import { applyDirection } from './i18n';
import { APP_NAME } from '../../bridge/src';
import './styles.css';
import './screens/preview.css';

document.title = APP_NAME;
applyDirection();

const route = window.location.hash.replace(/^#\/?/, '').split('?')[0];
const overlay = route === 'dock' || route === 'compact' || route === 'menu';
if (overlay) document.documentElement.classList.add('overlay-window');

const screens = import.meta.glob('./screens/*.tsx');

/** `#/preview/settings` mounts the default export of a matching screen file. */
function Preview({ name }: { name: string }) {
  const [Screen, setScreen] = useState<null | ((props: ScreenProps) => ReactNode)>(null);
  useEffect(() => {
    const wanted = name.toLowerCase().replace(/screen$/, '');
    const key = Object.keys(screens).find(path => {
      const file = path.split('/').pop()!.replace(/\.tsx$/, '').toLowerCase().replace(/screen$/, '');
      return file === wanted && !['app', 'gallery', 'previewhost', 'shared', 'overlaywindows'].includes(file);
    });
    if (!key) return;
    void (screens[key] as () => Promise<{ default?: (props: ScreenProps) => ReactNode }>)( ).then(module => {
      if (module.default) setScreen(() => module.default!);
    });
  }, [name]);
  if (!Screen) return <div className="preview-missing">No screen named “{name}”. Files are mounted by their default export.</div>;
  return <PreviewHost>{props => <Screen {...props} />}</PreviewHost>;
}

const preview = route.startsWith('preview/') ? route.slice('preview/'.length) : null;
const screen = route === 'dock' ? <DockWindow /> : route === 'compact' ? <CompactWindow /> : route === 'menu' ? <MenuWindow /> : route === 'gallery' ? <Gallery /> : preview ? <Preview name={preview} /> : <App />;
createRoot(document.getElementById('root')!).render(<React.StrictMode>{screen}</React.StrictMode>);
