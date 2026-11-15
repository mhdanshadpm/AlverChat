import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import type { Update } from '@tauri-apps/plugin-updater';
import { APP_NAME, type AppSettings, type BackupResult, type ConnectionReport, type PermissionStatus } from '../../../bridge/src';
import desktopConfig from '../../../../apps/desktop/src-tauri/tauri.conf.json';
import {
  AVATAR_PALETTE,
  Avatar,
  Badge,
  Banner,
  Button,
  Dialog,
  EmptyState,
  HotkeyRecorder,
  Icon,
  Input,
  ListGroup,
  ListRow,
  Select,
  Slider,
  ThemeCards,
  Toggle,
  WeekdayPicker,
  formatFileSize,
  statusLineFor,
  type IconName,
  type SelectOption,
  type ThemeChoice,
  type AvatarPaletteColor,
} from '../components';
import { LANGUAGES, ns, registerNamespace } from '../i18n';
import settingsCatalog from '../i18n/en/settings';
import { namedDevices, openMedia, probeDevices } from '../media';
import { installAndRestart } from '../updater';
import { ringPattern } from '../ringtone';
import { ScreenHeader, type ScreenProps } from './shared';
import { themeChoiceFor } from '../components/ThemeCards';
import { displayChord, platformChord } from '../components/HotkeyRecorder';
import './SettingsScreen.css';

registerNamespace('settings', settingsCatalog);

/* ------------------------------------------------------------------ *
 * 7.10 Settings — left category rail plus every control on
 * design/AlverChat_Settings_Design_System.png. Every widget is bound to a real
 * `AppSettings` field or a real bridge call; nothing here is a stub.
 * ------------------------------------------------------------------ */

/* ===== logic ===== */

/**
 * Read from the desktop build config rather than typed a second time: that file's `version` is what
 * `tauri build` stamps on the artifacts and what the updater compares a manifest against, so a bump
 * there has to move the number the About page prints. `release_config_tests` refuses a literal here.
 */
export const APP_VERSION: string = desktopConfig.version;

/**
 * The shortest backup passphrase the desktop accepts. `apps/desktop/src-tauri/src/platform.rs`
 * owns that gate, and `settings_surface_tests` refuses this copy once it drifts from it.
 */
const BACKUP_PASSPHRASE_MIN_LEN = 8;

export type SettingsCategory =
  | 'general'
  | 'icon'
  | 'notifications'
  | 'messages'
  | 'calls'
  | 'privacy'
  | 'about';

/** Rail order, straight off the board. */
export const CATEGORY_ORDER: readonly SettingsCategory[] = [
  'general',
  'icon',
  'notifications',
  'messages',
  'calls',
  'privacy',
  'about',
];

export const CATEGORY_ICONS: Record<SettingsCategory, IconName> = {
  general: 'settings',
  icon: 'messageCircle',
  notifications: 'bell',
  messages: 'messageCircle',
  calls: 'phone',
  privacy: 'shieldCheck',
  about: 'info',
};

/** `5` → `5 s`, the read-out the sliders show on the board. */
export function secondsLabel(value: number): string {
  return `${value} s`;
}

/** `90` → `90%`. */
export function percentLabel(value: number): string {
  return `${value}%`;
}

/** `22:00` → the locale's short form (`10:00 PM` in en). */
export function clockLabel(value: string): string {
  const [hourPart, minutePart] = value.split(':');
  const hour = Number(hourPart);
  const minute = Number(minutePart);
  if (!Number.isInteger(hour) || !Number.isInteger(minute) || hour > 23 || minute > 59) return value;
  return new Date(2000, 0, 1, hour, minute).toLocaleTimeString([], {
    hour: 'numeric',
    minute: '2-digit',
  });
}

/** Hourly `HH:MM` choices — the format `minutes_of_day()` parses on the Rust side. */
export function hourOptions(): SelectOption[] {
  return Array.from({ length: 24 }, (_, hour) => {
    const value = `${String(hour).padStart(2, '0')}:00`;
    return { value, label: clockLabel(value) };
  });
}

/** Number-valued settings rendered as a dropdown of `{ value, label }` pairs. */
export function numberOptions(values: readonly number[], labelFor: (value: number) => string): SelectOption[] {
  return values.map((value) => ({ value: String(value), label: labelFor(value) }));
}

/** The six corners/sides the dock can sit on; `left*` drives the mock's side. */
export const CORNER_VALUES = [
  'left-top',
  'left-center',
  'left-bottom',
  'right-top',
  'right-center',
  'right-bottom',
] as const;

/** Exported so the key guard can resolve each corner label against `settings`. */
export const CORNER_LABELS: Record<(typeof CORNER_VALUES)[number], [string, string]> = {
  'left-top': ['cornerLeftTop', 'Left - Top'],
  'left-center': ['cornerLeftCenter', 'Left - Center'],
  'left-bottom': ['cornerLeftBottom', 'Left - Bottom'],
  'right-top': ['cornerRightTop', 'Right - Top'],
  'right-center': ['cornerRightCenter', 'Right - Center'],
  'right-bottom': ['cornerRightBottom', 'Right - Bottom'],
};

export function cornerValueFor(value: string): string {
  return (CORNER_VALUES as readonly string[]).includes(value) ? value : 'right-center';
}

/** Device choices for one `enumerateDevices()` kind, with a Default first. */
export function deviceOptions(
  devices: readonly MediaDeviceInfo[],
  kind: MediaDeviceKind,
  defaultLabel: string,
): SelectOption[] {
  const named = namedDevices(devices)
    .filter((device) => device.kind === kind)
    .map((device) => ({ value: device.deviceId, label: device.label }));
  return [{ value: 'default', label: defaultLabel }, ...named];
}

/** Keeps a stored deviceId selectable even when it is not in the current list. */
export function withCurrent(
  options: readonly SelectOption[],
  value: string,
  label: string,
): SelectOption[] {
  if (!value || value === 'default' || options.some((option) => option.value === value)) {
    return [...options];
  }
  return [...options, { value, label }];
}

/** Adapter choices: automatic plus whatever the last connection test saw. */
export function adapterOptions(adapters: readonly string[], autoLabel: string): SelectOption[] {
  return [{ value: 'auto', label: autoLabel }, ...adapters.map((adapter) => ({ value: adapter, label: adapter }))];
}

/** Last path segment of a backup/export location. */
export function fileNameFor(path: string): string {
  const parts = path.split(/[\\/]/).filter(Boolean);
  return parts[parts.length - 1] ?? path;
}

/** Backup timestamp for the results list. */
export function dateLabel(createdMs: number): string {
  return new Date(createdMs).toLocaleString([], {
    year: 'numeric',
    month: 'short',
    day: 'numeric',
    hour: 'numeric',
    minute: '2-digit',
  });
}

/** Peer colours are plain strings on the bridge; Avatar wants the palette. */
export function paletteColorFor(color: string | undefined): AvatarPaletteColor | undefined {
  return AVATAR_PALETTE.includes(color as AvatarPaletteColor) ? (color as AvatarPaletteColor) : undefined;
}

/** Chords are stored per platform but always displayed in canonical form. */
export function hotkeyDisplay(value: string): string {
  return displayChord(value);
}

export function isMacPlatform(): boolean {
  if (typeof navigator === 'undefined') return false;
  return /mac/i.test(navigator.platform || navigator.userAgent);
}

/** `CmdOrCtrl+Alt+S` in, `Command+Alt+S` / `Ctrl+Alt+S` out. */
export function hotkeyForPlatform(value: string, isMac = isMacPlatform()): string {
  return platformChord(value, isMac);
}

/* ===== component ===== */

export default function SettingsScreen(props: ScreenProps) {
  const { bridge, peers, settings, patchSettings, toast, identity } = props;
  const text = useMemo(() => ns('settings'), []);
  const [category, setCategory] = useState<SettingsCategory>('general');

  const [adapters, setAdapters] = useState<string[]>([]);
  const [report, setReport] = useState<ConnectionReport | null>(null);
  const [testingNetwork, setTestingNetwork] = useState(false);
  const [guideSteps, setGuideSteps] = useState<string[]>([]);
  const [dndActive, setDndActive] = useState(false);
  const [passphraseSet, setPassphraseSet] = useState(false);
  const [passphrase, setPassphrase] = useState('');
  const [showPassphrase, setShowPassphrase] = useState(false);
  const [devices, setDevices] = useState<MediaDeviceInfo[]>([]);
  const [micPermission, setMicPermission] = useState<PermissionStatus | null>(null);
  const [cameraPermission, setCameraPermission] = useState<PermissionStatus | null>(null);
  const [testingSetup, setTestingSetup] = useState(false);
  const [probing, setProbing] = useState(false);
  const [setupResult, setSetupResult] = useState<{ ok: boolean; lines: string[] } | null>(null);
  const [backups, setBackups] = useState<BackupResult[]>([]);
  const [exportResult, setExportResult] = useState<{ path: string; messages: number } | null>(null);
  const [busy, setBusy] = useState<'backup' | 'export' | 'restore' | 'update' | null>(null);
  const [updateState, setUpdateState] = useState<'unknown' | 'current' | 'available'>('unknown');
  const [pendingUpdate, setPendingUpdate] = useState<Update | null>(null);
  const [backupDialog, setBackupDialog] = useState(false);
  const [restoreDialog, setRestoreDialog] = useState(false);
  const [mutedDialog, setMutedDialog] = useState(false);
  const [clearDialog, setClearDialog] = useState(false);
  const [backupPassphrase, setBackupPassphrase] = useState('');
  const [restorePath, setRestorePath] = useState('');
  const [restorePassphrase, setRestorePassphrase] = useState('');
  const audioRef = useRef<AudioContext | null>(null);

  const update = useCallback((patch: Partial<AppSettings>) => patchSettings(patch), [patchSettings]);

  const fail = useCallback(
    (error: unknown) => toast(String(error instanceof Error ? error.message : error), 'danger'),
    [toast],
  );

  /* --- bridge reads ---------------------------------------------------- */
  useEffect(() => {
    let mounted = true;
    void bridge
      .runConnectionTest()
      .then((result) => {
        if (mounted) setAdapters(result.adapters);
      })
      .catch(() => {
        /* outside the desktop app there is nothing to bind to */
      });
    return () => {
      mounted = false;
    };
  }, [bridge]);

  useEffect(() => {
    let mounted = true;
    void bridge
      .openFirewallGuide()
      .then((steps) => {
        if (mounted) setGuideSteps(steps);
      })
      .catch(fail);
    return () => {
      mounted = false;
    };
  }, [bridge, fail]);

  useEffect(() => {
    let mounted = true;
    void bridge
      .hasLanPassphrase()
      .then((value) => {
        if (mounted) setPassphraseSet(value);
      })
      .catch(() => {
        if (mounted) setPassphraseSet(false);
      });
    return () => {
      mounted = false;
    };
  }, [bridge, settings.lanEncryption]);

  useEffect(() => {
    let mounted = true;
    void bridge
      .isDndActive()
      .then((value) => {
        if (mounted) setDndActive(value);
      })
      .catch(() => {
        if (mounted) setDndActive(false);
      });
    return () => {
      mounted = false;
    };
  }, [bridge, settings.dndEnabled, settings.dndDays, settings.dndFrom, settings.dndTo]);

  useEffect(() => {
    let mounted = true;
    void bridge
      .listBackups()
      .then((list) => {
        if (mounted) setBackups(list);
      })
      .catch(() => {
        if (mounted) setBackups([]);
      });
    return () => {
      mounted = false;
    };
  }, [bridge]);

  useEffect(() => {
    let mounted = true;
    void Promise.all([bridge.permissionStatus('microphone'), bridge.permissionStatus('camera')])
      .then(([mic, camera]) => {
        if (!mounted) return;
        setMicPermission(mic);
        setCameraPermission(camera);
      })
      .catch(() => {
        /* permission kinds the platform does not know about are simply unknown */
      });
    return () => {
      mounted = false;
    };
  }, [bridge]);

  const refreshDevices = useCallback((): Promise<readonly MediaDeviceInfo[]> => {
    const media = typeof navigator !== 'undefined' ? navigator.mediaDevices : undefined;
    if (!media?.enumerateDevices) return Promise.resolve([]);
    return media
      .enumerateDevices()
      .then((list) => {
        setDevices(list);
        return list as readonly MediaDeviceInfo[];
      })
      .catch(() => [] as readonly MediaDeviceInfo[]);
  }, []);

  useEffect(() => {
    if (micPermission?.state === 'denied') return;
    const media = typeof navigator !== 'undefined' ? navigator.mediaDevices : undefined;
    void refreshDevices();
    if (!media?.addEventListener) return;
    /* Plugging in a headset or starting a call both rename the whole list. */
    const onDeviceChange = () => {
      void refreshDevices();
    };
    media.addEventListener('devicechange', onDeviceChange);
    return () => media.removeEventListener('devicechange', onDeviceChange);
  }, [micPermission?.state, refreshDevices, settings.microphone, settings.camera]);

  /* --- derived --------------------------------------------------------- */
  const mutedChats = useMemo(() => peers.filter((peer) => peer.muted), [peers]);
  const onlineCount = useMemo(() => peers.filter((peer) => peer.online).length, [peers]);

  /* --- actions --------------------------------------------------------- */
  const changeLaunchAtLogin = useCallback(
    (enabled: boolean) => {
      void bridge
        .setLaunchAtLogin(enabled)
        .then((applied) => update({ launchAtLogin: applied }))
        .catch(fail);
    },
    [bridge, fail, update],
  );

  const changeSnapToEdge = useCallback(
    (enabled: boolean) => {
      void bridge.setSnapToEdge(enabled).catch(fail);
      update({ snapToEdge: enabled });
    },
    [bridge, fail, update],
  );

  const changeBubbles = useCallback(
    (enabled: boolean) => {
      void bridge.setBubbleSettings(enabled, settings.bubbleAutoHideSeconds).catch(fail);
      update({ bubblesEnabled: enabled });
    },
    [bridge, fail, settings.bubbleAutoHideSeconds, update],
  );

  const changeAutoHide = useCallback(
    (seconds: number) => {
      update({ bubbleAutoHideSeconds: seconds });
      void bridge.setBubbleSettings(settings.bubblesEnabled, seconds).catch(fail);
    },
    [bridge, fail, settings.bubblesEnabled, update],
  );

  const changeHotkey = useCallback(
    (combo: string) => {
      if (!combo) {
        update({ hotkey: '' });
        toast(text('hotkeyCleared', 'Shortcut cleared. The previous one stays active until you set a new one.'));
        return;
      }
      void bridge
        .setHotkey(hotkeyForPlatform(combo))
        .then((registered) => {
          update({ hotkey: registered });
          toast(text('hotkeySaved', 'Shortcut saved as {{combo}}.', { combo: hotkeyDisplay(registered) }), 'success');
        })
        .catch(fail);
    },
    [bridge, fail, text, toast, update],
  );

  const resetPosition = useCallback(() => {
    void bridge
      .resetOverlayPosition()
      .then(() => toast(text('positionReset', 'The floating icon is back in its default spot.'), 'success'))
      .catch(fail);
  }, [bridge, fail, text, toast]);

  const playTone = useCallback(
    (kind: 'notification' | 'ringtone') => {
      const Ctor =
        typeof window !== 'undefined'
          ? window.AudioContext ??
            (window as unknown as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext
          : undefined;
      if (!Ctor) {
        toast(text('audioBlocked', 'This window blocked audio playback.'), 'danger');
        return;
      }
      try {
        const ctx = audioRef.current ?? new Ctor();
        audioRef.current = ctx;
        const notes =
          kind === 'ringtone'
            ? ringPattern(settings.ringtone).notes
            : [
                { at: 0, hz: 880 },
                { at: 0.19, hz: 1174.66 },
              ];
        notes.forEach((note) => {
          const start = ctx.currentTime + note.at;
          const oscillator = ctx.createOscillator();
          const gain = ctx.createGain();
          oscillator.type = 'sine';
          oscillator.frequency.value = note.hz;
          gain.gain.setValueAtTime(0.0001, start);
          gain.gain.exponentialRampToValueAtTime(0.09, start + 0.02);
          gain.gain.exponentialRampToValueAtTime(0.0001, start + 0.17);
          oscillator.connect(gain);
          gain.connect(ctx.destination);
          oscillator.start(start);
          oscillator.stop(start + 0.18);
        });
        void ctx.resume();
      } catch (error) {
        fail(error);
      }
    },
    [fail, settings.ringtone, text, toast],
  );

  const runTest = useCallback(() => {
    setTestingNetwork(true);
    void bridge
      .runConnectionTest()
      .then((result) => {
        setReport(result);
        setAdapters(result.adapters);
        toast(
          result.listening
            ? text('testDone', 'Listening on port {{port}} with {{peers}} peers.', {
                port: result.port,
                peers: result.peers,
              })
            : text('testSilent', 'AlverChat is not listening on the network right now.'),
          result.warnings.length > 0 ? 'danger' : 'success',
        );
      })
      .catch(fail)
      .finally(() => setTestingNetwork(false));
  }, [bridge, fail, text, toast]);

  const openGuide = useCallback(() => {
    void bridge
      .openFirewallGuide()
      .then((steps) => {
        setGuideSteps(steps);
        toast(text('guideOpened', 'Firewall steps are listed below.'));
      })
      .catch(fail);
  }, [bridge, fail, text, toast]);

  const chooseFolder = useCallback(() => {
    if (!settings.downloadsDir.trim()) {
      toast(text('folderMissing', 'Type a folder path first, then open it to confirm.'));
      return;
    }
    void bridge.revealPath(settings.downloadsDir).catch(fail);
  }, [bridge, fail, settings.downloadsDir, toast]);

  /**
   * Device names are invisible to this webview until it holds media access, so a
   * brief grant is the only way to turn an empty picker into a usable one.
   */
  const detectDevices = useCallback(() => {
    const media = typeof navigator !== 'undefined' ? navigator.mediaDevices : undefined;
    if (!media?.getUserMedia) {
      toast(text('noMedia', 'This window does not expose camera or microphone access.'), 'danger');
      return;
    }
    setProbing(true);
    void probeDevices(media)
      .then(async (probe) => {
        const list = await refreshDevices();
        const inputs = namedDevices(list).filter((device) => device.kind !== 'audiooutput').length;
        if (probe === 'none') {
          toast(
            text('detectNone', 'AlverChat needs microphone and camera access before it can list devices.'),
            'danger',
          );
        } else if (inputs === 0) {
          toast(text('detectEmpty', 'Access is allowed, but no camera or microphone was reported.'), 'danger');
        } else {
          toast(
            probe === 'both'
              ? text('detectOk', '{{count}} devices ready — choose your microphone and camera below.', {
                  count: inputs,
                })
              : text('detectMicOnly', '{{count}} microphones ready. Allow camera access to choose a camera.', {
                  count: inputs,
                }),
            'success',
          );
        }
      })
      .finally(() => setProbing(false));
  }, [refreshDevices, text, toast]);

  const testSetup = useCallback(async () => {
    setTestingSetup(true);
    setSetupResult(null);
    const lines: string[] = [];
    const media = typeof navigator !== 'undefined' ? navigator.mediaDevices : undefined;
    try {
      if (!media?.getUserMedia) {
        throw new Error(text('noMedia', 'This window does not expose camera or microphone access.'));
      }
      if (micPermission?.state === 'denied' || cameraPermission?.state === 'denied') {
        throw new Error(
          text('permissionDenied', 'Your system is blocking camera or microphone access for AlverChat.'),
        );
      }
      const audioId = settings.microphone === 'default' ? undefined : settings.microphone;
      const videoId = settings.camera === 'default' ? undefined : settings.camera;
      // The same pins a call uses, so passing here means calling works too.
      const opened = await openMedia(media, {
        audio: {
          deviceId: audioId ? { exact: audioId } : undefined,
          echoCancellation: settings.echoCancellation,
          noiseSuppression: settings.noiseSuppression,
        },
        video: videoId ? { deviceId: { exact: videoId } } : true,
      });
      const sawVideo = opened.stream.getVideoTracks().length > 0;
      opened.stream.getTracks().forEach((track) => track.stop());
      const list = namedDevices(await refreshDevices());
      lines.push(
        text('testMic', 'Microphone “{{device}}” captured audio.', {
          device:
            list.find((device) => device.deviceId === settings.microphone)?.label ??
            text('defaultDevice', 'the default device'),
        }),
      );
      if (opened.usedDefault) {
        lines.push(text('deviceGone', 'The device you chose was not connected, so the system default was used instead.'));
      }
      if (sawVideo) lines.push(text('testCamera', 'Camera preview started and stopped cleanly.'));
      lines.push(
        text('testSpeakers', '{{count}} output device(s) available.', {
          count: list.filter((device) => device.kind === 'audiooutput').length,
        }),
      );
      setSetupResult({ ok: true, lines });
      toast(text('testOk', 'Your call setup is working.'), 'success');
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setSetupResult({ ok: false, lines: [message] });
      toast(text('testFailed', 'Call test failed: {{message}}', { message }), 'danger');
    } finally {
      setTestingSetup(false);
    }
  }, [
    cameraPermission?.state,
    micPermission?.state,
    refreshDevices,
    settings.camera,
    settings.echoCancellation,
    settings.microphone,
    settings.noiseSuppression,
    text,
    toast,
  ]);

  const unmuteChat = useCallback(
    (chatId: string) => {
      void bridge
        .setMutedChat(chatId, false)
        .then((muted) => update({ mutedChats: muted }))
        .catch(fail);
    },
    [bridge, fail, update],
  );

  const toggleChatMute = useCallback(
    (chatId: string, muted: boolean) => {
      void bridge
        .setMutedChat(chatId, muted)
        .then((list) => {
          update({ mutedChats: list });
        })
        .catch(fail);
    },
    [bridge, fail, update],
  );

  const savePassphrase = useCallback(() => {
    void bridge
      .setLanPassphrase(passphrase)
      .then((stored) => {
        setPassphraseSet(stored);
        update({ lanEncryption: stored });
        setPassphrase('');
        toast(
          stored
            ? text('passphraseSaved', 'LAN passphrase saved.')
            : text('passphraseCleared', 'LAN passphrase removed from this device.'),
          stored ? 'success' : 'neutral',
        );
      })
      .catch(fail);
  }, [bridge, fail, passphrase, text, toast, update]);

  const createBackupNow = useCallback(() => {
    setBusy('backup');
    void bridge
      .createBackup(backupPassphrase)
      .then((result) => {
        setBackups((current) => [result, ...current]);
        setBackupDialog(false);
        setBackupPassphrase('');
        toast(
          text('backupDone', 'Backup written to {{name}} ({{size}}).', {
            name: fileNameFor(result.path),
            size: formatFileSize(result.bytes),
          }),
          'success',
        );
      })
      .catch(fail)
      .finally(() => setBusy(null));
  }, [backupPassphrase, bridge, fail, text, toast]);

  const restoreNow = useCallback(() => {
    if (!restorePath) {
      toast(text('restorePick', 'Choose a backup to restore first.'));
      return;
    }
    setBusy('restore');
    void bridge
      .restoreBackup(restorePath, restorePassphrase)
      .then(() => {
        setRestoreDialog(false);
        setRestorePassphrase('');
        toast(text('restoreDone', 'Backup restored. Restart AlverChat to see every chat.'), 'success');
      })
      .catch(fail)
      .finally(() => setBusy(null));
  }, [bridge, fail, restorePassphrase, restorePath, text, toast]);

  const exportNow = useCallback(() => {
    setBusy('export');
    void bridge
      .exportData()
      .then((result) => {
        setExportResult(result);
        toast(
          text('exportDone', 'Exported {{count}} messages to {{name}}.', {
            count: result.messages,
            name: fileNameFor(result.path),
          }),
          'success',
        );
      })
      .catch(fail)
      .finally(() => setBusy(null));
  }, [bridge, fail, text, toast]);

  const clearHistory = useCallback(async () => {
    setClearDialog(false);
    let removed = 0;
    try {
      for (const chat of peers) {
        const messages = await bridge.listMessages(chat.id);
        for (const message of messages) {
          if (await bridge.deleteMessageLocal(message.id)) removed += 1;
        }
      }
      toast(
        text('historyCleared', 'Removed {{count}} messages from this device.', { count: removed }),
        removed > 0 ? 'success' : 'neutral',
      );
    } catch (error) {
      fail(error);
    }
  }, [bridge, fail, peers, text, toast]);

  const checkUpdates = useCallback(async () => {
    setBusy('update');
    setPendingUpdate(null);
    try {
      if (typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window) {
        const { check } = await import('@tauri-apps/plugin-updater');
        const updateInfo = await check();
        setPendingUpdate(updateInfo);
        setUpdateState(updateInfo ? 'available' : 'current');
        toast(
          updateInfo
            ? text('updateFound', 'Version {{version}} is available to install.', { version: updateInfo.version })
            : text('upToDate', 'AlverChat {{version}} is up to date.', { version: APP_VERSION }),
          updateInfo ? 'accent' : 'success',
        );
      } else {
        setUpdateState('current');
        toast(
          text('updateDesktopOnly', 'Update checks only run inside the AlverChat desktop app.'),
          'neutral',
        );
      }
    } catch (error) {
      fail(error);
    } finally {
      setBusy(null);
    }
  }, [fail, text, toast]);

  /**
   * The half the docs have described since Phase 7 and no code performed: the download, the
   * signature check against the key bundled in `tauri.conf.json`, and the restart. `check` only
   * reports, so nothing downloads until a person clicks this row.
   */
  const installUpdate = useCallback(async () => {
    if (!pendingUpdate) return;
    setBusy('update');
    try {
      toast(
        text('updateInstalling', 'Installing {{version}}. AlverChat restarts when it is done.', { version: pendingUpdate.version }),
        'neutral',
      );
      const { relaunch } = await import('@tauri-apps/plugin-process');
      await installAndRestart(pendingUpdate, relaunch);
    } catch (error) {
      // A refused signature arrives here rather than as a dialog, which is the whole reason the
      // key is checked before anything is written to disk.
      fail(error);
    } finally {
      setBusy(null);
    }
  }, [fail, pendingUpdate, text, toast]);

  /* --- option lists ---------------------------------------------------- */
  const languageLabels: Record<string, string> = { en: text('languageEnglish', 'English') };
  /* Offered from what i18n serves, so the picker cannot name a language the app has no strings for. */
  const languageOptions: SelectOption[] = LANGUAGES.map(value => ({ value, label: languageLabels[value] ?? value }));
  const themeLabels = {
    light: text('themeLight', 'Light'),
    dark: text('themeDark', 'Dark'),
    system: text('themeSystem', 'System'),
  };
  const idleIconOptions: SelectOption[] = [
    { value: 'logo', label: text('idleLogo', 'AlverChat Logo') },
    { value: 'photo', label: text('idlePhoto', 'My Profile Photo') },
  ];
  const cornerOptions: SelectOption[] = CORNER_VALUES.map((value) => {
    const [key, fallback] = CORNER_LABELS[value];
    return { value, label: text(key, fallback) };
  });
  const editOptions = numberOptions([15, 60, 1440, 0], (value) =>
    value === 0
      ? text('editNever', 'No limit')
      : value < 60
        ? text('editMinutes', '{{count}} minutes', { count: value })
        : value === 60
          ? text('editHour', '{{count}} hour', { count: value / 60 })
          : text('editDay', '{{count}} hours', { count: value / 60 }),
  );
  const downloadOptions = numberOptions([10, 25, 50, 100, 250], (value) =>
    text('sizeMb', '{{count}} MB', { count: value }),
  );
  const fileOptions = numberOptions([100, 250, 500, 1000, 2000], (value) =>
    value < 1000 ? text('sizeMb', '{{count}} MB', { count: value }) : text('sizeGb', '{{count}} GB', { count: value / 1000 }),
  );
  const ringtoneOptions: SelectOption[] = [
    { value: 'alverchat', label: text('ringAlverchat', 'AlverChat Call') },
    { value: 'chime', label: text('ringChime', 'Soft Chime') },
    { value: 'none', label: text('ringNone', 'None') },
  ];
  const statusOptions: SelectOption[] = [
    { value: 'everyone', label: text('statusEveryone', 'Everyone on my network') },
    { value: 'nobody', label: text('statusNobody', 'Nobody') },
  ];
  const defaultMic = text('defaultMic', 'Default microphone');
  const defaultSpeaker = text('defaultSpeaker', 'Default speakers');
  const defaultCamera = text('defaultCamera', 'Default camera');
  const savedDevice = text('savedDevice', 'Saved device (not connected now)');
  const micOptions = withCurrent(deviceOptions(devices, 'audioinput', defaultMic), settings.microphone, savedDevice);
  const speakerOptions = withCurrent(
    deviceOptions(devices, 'audiooutput', defaultSpeaker),
    settings.speaker,
    savedDevice,
  );
  const cameraOptions = withCurrent(deviceOptions(devices, 'videoinput', defaultCamera), settings.camera, savedDevice);
  const networkOptions = adapterOptions(adapters, text('adapterAuto', 'Automatic'));

  /* --- rail ------------------------------------------------------------ */
  const railItems: Array<{ id: SettingsCategory; label: string }> = [
    { id: 'general', label: text('catGeneral', 'General') },
    { id: 'icon', label: text('catIcon', 'Floating Icon & Bubbles') },
    { id: 'notifications', label: text('catNotifications', 'Notifications') },
    { id: 'messages', label: text('catMessages', 'Messages') },
    { id: 'calls', label: text('catCalls', 'Calls') },
    { id: 'privacy', label: text('catPrivacy', 'Privacy & Network') },
    { id: 'about', label: text('catAbout', 'About') },
  ];

  const heads: Record<SettingsCategory, { title: string; description: string }> = {
    general: {
      title: text('catGeneral', 'General'),
      description: text('headGeneral', 'App behavior, appearance and basic preferences.'),
    },
    icon: {
      title: text('catIcon', 'Floating Icon & Bubbles'),
      description: text('headIcon', 'Control the floating icon and message bubbles.'),
    },
    notifications: {
      title: text('catNotifications', 'Notifications'),
      description: text('headNotifications', 'Manage notification sounds and alerts.'),
    },
    messages: {
      title: text('catMessages', 'Messages'),
      description: text('headMessages', 'Message behavior, downloads and history.'),
    },
    calls: {
      title: text('catCalls', 'Calls'),
      description: text('headCalls', 'Audio and video call preferences.'),
    },
    privacy: {
      title: text('catPrivacy', 'Privacy & Network'),
      description: text('headPrivacy', 'Security, visibility and LAN connectivity.'),
    },
    about: {
      title: text('catAbout', 'About'),
      description: text('headAbout', 'App info, updates, backup and data export.'),
    },
  };

  const row = (
    key: string,
    icon: IconName,
    title: string,
    control: ReactNode,
    hint?: string,
  ): ReactNode => (
    <ListRow
      className="stg-row"
      hint={hint}
      icon={icon}
      key={key}
      title={title}
      trailing={control}
    />
  );

  const toggleFor = (
    label: string,
    value: boolean,
    onChange: (next: boolean) => void,
  ): ReactNode => <Toggle checked={value} label={label} onChange={onChange} />;

  const selectFor = (
    label: string,
    value: string,
    options: SelectOption[],
    onChange: (next: string) => void,
  ): ReactNode => (
    <Select
      aria-label={label}
      className="stg-select"
      onChange={(event) => onChange(event.target.value)}
      options={options}
      value={value}
    />
  );

  const sliderFor = (
    label: string,
    value: number,
    min: number,
    max: number,
    onChange: (next: number) => void,
    step = 1,
  ): ReactNode => (
    <Slider
      className="stg-slider"
      format={secondsLabel}
      label={label}
      max={max}
      min={min}
      onChange={onChange}
      step={step}
      value={value}
    />
  );

  return (
    <div className="stg">
      <nav aria-label={text('categories', 'Settings categories')} className="stg-rail">
        <div className="stg-rail-head">
          <span className="stg-rail-mark">
            <Icon name="messageCircle" size={16} filled />
          </span>
          <b>{APP_NAME}</b>
          <i aria-hidden className="stg-rail-rule" />
          <span>{text('title', 'Settings')}</span>
        </div>
        {/* Becomes a sideways strip below 980 px; the flag tells the layout audit it
            is deliberate rather than an overflow. */}
        <div className="stg-rail-list" data-ac-scroll-x="">
          {railItems.map((item) => (
            <ListRow
              current={category === item.id}
              active={category === item.id}
              className="stg-rail-item"
              icon={CATEGORY_ICONS[item.id]}
              key={item.id}
              onClick={() => setCategory(item.id)}
              title={item.label}
            />
          ))}
        </div>
        <div className="stg-rail-foot">
          <span className="stg-rail-version">
            {APP_NAME} v{APP_VERSION}
          </span>
          <span className="stg-rail-status">
            <i className={onlineCount > 0 ? 'stg-dot stg-dot--on' : 'stg-dot'} />
            {onlineCount > 0
              ? text('connectedLan', 'Connected (LAN)')
              : text('searchingLan', 'Searching the network')}
          </span>
        </div>
      </nav>

      <div className="stg-content">
        <ScreenHeader
          description={heads[category].description}
          eyebrow={text('eyebrow', 'Preferences')}
          title={heads[category].title}
        />

        {category === 'general' ? (
          <div className="stg-stack">
            <ListGroup>
              {row(
                'displayName',
                'user',
                text('displayName', 'Display Name'),
                <Input
                  aria-label={text('displayName', 'Display Name')}
                  className="stg-input stg-input--name"
                  maxLength={40}
                  onChange={(event) => update({ displayName: event.target.value })}
                  placeholder={text('displayNamePlaceholder', 'Enter a display name')}
                  value={settings.displayName}
                />,
                text('displayNameHint', 'This name will be shown to others in chats, calls and groups.'),
              )}
              {row(
                'launchAtLogin',
                'power',
                text('launchAtLogin', 'Launch at Login'),
                toggleFor(text('launchAtLogin', 'Launch at Login'), settings.launchAtLogin, changeLaunchAtLogin),
                text('launchAtLoginHint', 'Start AlverChat automatically when I log in.'),
              )}
              {row(
                'language',
                'languages',
                text('language', 'Language'),
                selectFor(text('language', 'Language'), settings.language, languageOptions, (next) =>
                  update({ language: next }),
                ),
                text('languageHint', 'English for now; more languages are planned.'),
              )}
              <ListRow
                className="stg-row"
                hint={text('themeHint', 'Choose how AlverChat looks.')}
                icon="palette"
                title={text('theme', 'Theme')}
              />
              <div className="stg-theme">
                <ThemeCards
                  label={text('themeGroup', 'Theme')}
                  labels={themeLabels}
                  onChange={(next: ThemeChoice) => update({ theme: next })}
                  value={themeChoiceFor(settings.theme)}
                />
              </div>
            </ListGroup>
          </div>
        ) : null}

        {category === 'icon' ? (
          <div className="stg-stack">
            <ListGroup>
              {row(
                'bubbles',
                'messageCircle',
                text('bubbles', 'Enable message bubbles'),
                toggleFor(text('bubbles', 'Enable message bubbles'), settings.bubblesEnabled, changeBubbles),
                text('bubblesHint', 'Show new messages as pop-up bubbles.'),
              )}
              {row(
                'idleIcon',
                'image',
                text('idleIcon', 'Idle icon'),
                selectFor(text('idleIcon', 'Idle icon'), settings.idleIcon, idleIconOptions, (next) =>
                  update({ idleIcon: next as AppSettings['idleIcon'] }),
                ),
              )}
              {row(
                'morph',
                'sparkles',
                text('morph', 'Icon morph (on activity)'),
                toggleFor(text('morph', 'Icon morph (on activity)'), settings.iconMorphEnabled, (next) =>
                  update({ iconMorphEnabled: next }),
                ),
                text('morphHint', 'Morph to the active state, e.g. unread or call.'),
              )}
              {row(
                'morphDelay',
                'timer',
                text('revertDelay', 'Revert delay'),
                sliderFor(
                  text('revertDelay', 'Revert delay'),
                  settings.iconMorphDelaySeconds,
                  1,
                  30,
                  (next) => update({ iconMorphDelaySeconds: next }),
                ),
              )}
              {row(
                'autoHide',
                'clock',
                text('autoHide', 'Bubble auto-hide delay'),
                sliderFor(text('autoHide', 'Bubble auto-hide delay'), settings.bubbleAutoHideSeconds, 2, 30, changeAutoHide),
              )}
              {row(
                'corner',
                'grid',
                text('corner', 'Screen corner / side'),
                selectFor(
                  text('corner', 'Screen corner / side'),
                  cornerValueFor(settings.screenCorner),
                  cornerOptions,
                  (next) => update({ screenCorner: next }),
                ),
              )}
              {row(
                'opacity',
                'layers',
                text('opacity', 'Opacity'),
                <Slider
                  className="stg-slider"
                  format={percentLabel}
                  label={text('opacity', 'Opacity')}
                  max={100}
                  min={20}
                  onChange={(next) => update({ opacityPercent: next })}
                  step={5}
                  value={settings.opacityPercent}
                />,
              )}
              {row(
                'snap',
                'move',
                text('snap', 'Snap to edge'),
                toggleFor(text('snap', 'Snap to edge'), settings.snapToEdge, changeSnapToEdge),
                text('snapHint', 'Dock the icon to the nearest screen edge.'),
              )}
              {row(
                'hotkey',
                'keyboard',
                text('hotkey', 'Show / hide hotkey'),
                <HotkeyRecorder
                  label={text('hotkey', 'Show / hide hotkey')}
                  onChange={changeHotkey}
                  placeholder={text('hotkeyPlaceholder', 'Press a shortcut')}
                  recordingLabel={text('hotkeyRecording', 'Recording…')}
                  clearLabel={text('hotkeyClear', 'Clear shortcut')}
                  value={settings.hotkey}
                />,
                text('hotkeyHint', 'Global shortcut that toggles the floating icon.'),
              )}
              {row(
                'resetPosition',
                'rotateCcw',
                text('resetPosition', 'Reset icon position'),
                <Button icon="move" onClick={resetPosition} size="sm" variant="secondary">
                  {text('resetPositionButton', 'Reset')}
                </Button>,
                text('resetPositionHint', 'Put the floating icon back at its default spot.'),
              )}
            </ListGroup>
            <p className="stg-note">
              {text('iconNote', 'The floating icon keeps running when this window is closed.')}
            </p>
          </div>
        ) : null}

        {category === 'notifications' ? (
          <div className="stg-stack">
            <ListGroup>
              {row(
                'sound',
                'volume2',
                text('sound', 'Notification sound'),
                <span className="stg-pair">
                  <Button icon="play" onClick={() => playTone('notification')} size="sm" variant="secondary">
                    {text('play', 'Play')}
                  </Button>
                  {toggleFor(text('sound', 'Notification sound'), settings.notificationSound, (next) =>
                    update({ notificationSound: next }),
                  )}
                </span>,
                text('soundHint', 'Play a sound when a message arrives.'),
              )}
              {row(
                'muteAll',
                'bellOff',
                text('muteAll', 'Mute all notifications'),
                toggleFor(text('muteAll', 'Mute all notifications'), settings.notificationsMuted, (next) =>
                  update({ notificationsMuted: next }),
                ),
                text('muteAllHint', 'Silence every alert until you turn this back on.'),
              )}
              <ListRow
                className="stg-row"
                hint={text('dndHint', 'Silence notifications during scheduled hours.')}
                icon="clock3"
                title={
                  <span className="stg-title-pair">
                    {text('dnd', 'Do Not Disturb (DND)')}
                    {dndActive ? <Badge tone="warning">{text('dndActive', 'Active now')}</Badge> : null}
                  </span>
                }
                trailing={toggleFor(text('dnd', 'Do Not Disturb (DND)'), settings.dndEnabled, (next) =>
                  update({ dndEnabled: next }),
                )}
              />
              <div className="stg-dnd">
                <div className="stg-dnd-label">{text('days', 'Days')}</div>
                <WeekdayPicker
                  legend={text('days', 'Days')}
                  onChange={(next) => update({ dndDays: next })}
                  value={settings.dndDays}
                />
                <div className="stg-dnd-grid">
                  <Select
                    aria-label={text('dndFrom', 'From')}
                    className="stg-select"
                    label={text('dndFrom', 'From')}
                    onChange={(event) => update({ dndFrom: event.target.value })}
                    options={hourOptions()}
                    value={settings.dndFrom}
                  />
                  <Select
                    aria-label={text('dndTo', 'To')}
                    className="stg-select"
                    label={text('dndTo', 'To')}
                    onChange={(event) => update({ dndTo: event.target.value })}
                    options={hourOptions()}
                    value={settings.dndTo}
                  />
                </div>
                <p className="stg-note">
                  {text(
                    'dndNote',
                    'Quiet hours can cross midnight, so {{from}} to {{to}} works overnight.',
                    { from: clockLabel(settings.dndFrom), to: clockLabel(settings.dndTo) },
                  )}
                </p>
              </div>
              {row(
                'preview',
                'messageCircle',
                text('preview', 'Show message preview'),
                toggleFor(text('preview', 'Show message preview'), settings.showMessagePreview, (next) =>
                  update({ showMessagePreview: next }),
                ),
                text('previewHint', 'Include the sender and message text in notifications.'),
              )}
            </ListGroup>

            <ListGroup section={`${text('mutedChats', 'Muted chats')} (${mutedChats.length})`}>
              {mutedChats.length === 0 ? (
                <EmptyState
                  compact
                  icon="bellOff"
                  message={text('mutedEmpty', 'Muted conversations appear here.')}
                  title={text('mutedEmptyTitle', 'Nothing is muted')}
                />
              ) : (
                mutedChats.map((peer) => (
                  <ListRow
                    className="stg-row"
                    hint={statusLineFor(peer)}
                    key={peer.id}
                    leading={
                      <Avatar
                        color={paletteColorFor(peer.color)}
                        group={peer.group}
                        initials={peer.initials}
                        name={peer.name}
                        size={24}
                      />
                    }
                    title={peer.name}
                    trailing={
                      <Button onClick={() => unmuteChat(peer.id)} size="sm" variant="secondary">
                        {text('unmute', 'Unmute')}
                      </Button>
                    }
                  />
                ))
              )}
              {row(
                'manageMuted',
                'slidersHorizontal',
                text('manageMuted', 'Manage muted chats'),
                <Button onClick={() => setMutedDialog(true)} size="sm" variant="secondary">
                  {text('manage', 'Manage')}
                </Button>,
                text('manageMutedHint', 'Pick which conversations stay quiet.'),
              )}
            </ListGroup>
          </div>
        ) : null}

        {category === 'messages' ? (
          <div className="stg-stack">
            <ListGroup>
              {row(
                'editLimit',
                'pencil',
                text('editLimit', 'Edit time limit'),
                selectFor(
                  text('editLimit', 'Edit time limit'),
                  String(settings.editWindowMinutes),
                  editOptions,
                  (next) => update({ editWindowMinutes: Number(next) }),
                ),
                text('editLimitHint', 'Time within which you can edit a message.'),
              )}
              {row(
                'autoDownload',
                'download',
                text('autoDownload', 'Auto-download size limit'),
                selectFor(
                  text('autoDownload', 'Auto-download size limit'),
                  String(settings.autoDownloadLimitMb),
                  downloadOptions,
                  (next) => update({ autoDownloadLimitMb: Number(next) }),
                ),
                text('autoDownloadHint', 'Automatically download media up to this size.'),
              )}
              {row(
                'maxFile',
                'hardDrive',
                text('maxFile', 'Max file size'),
                selectFor(
                  text('maxFile', 'Max file size'),
                  String(settings.maxFileSizeMb),
                  fileOptions,
                  (next) => update({ maxFileSizeMb: Number(next) }),
                ),
                text('maxFileHint', 'Largest file AlverChat will send or receive.'),
              )}
              <ListRow
                className="stg-row"
                hint={text('downloadsHint', 'Folder used for received files and media.')}
                icon="folder"
                title={text('downloads', 'Save downloads to')}
                trailing={
                  <span className="stg-pair stg-pair--wide">
                    <Input
                      aria-label={text('downloads', 'Save downloads to')}
                      className="stg-input stg-input--path"
                      onChange={(event) => update({ downloadsDir: event.target.value })}
                      placeholder={text('downloadsPlaceholder', 'Not set')}
                      value={settings.downloadsDir}
                    />
                    <Button icon="folder" onClick={chooseFolder} size="sm" variant="secondary">
                      {text('choose', 'Choose…')}
                    </Button>
                  </span>
                }
              />
              <ListRow
                className="stg-row"
                hint={text('clearHistoryHint', 'Delete all local chat messages and media from this device.')}
                icon="trash2"
                title={text('clearHistory', 'Clear chat history')}
                trailing={
                  <Button icon="trash2" onClick={() => setClearDialog(true)} size="sm" variant="danger">
                    {text('clearHistoryButton', 'Clear History')}
                  </Button>
                }
              />
            </ListGroup>
            <p className="stg-note">
              {text('messagesNote', 'Clearing history only touches this device — other people keep their copies.')}
            </p>
          </div>
        ) : null}

        {category === 'calls' ? (
          <div className="stg-stack">
            <ListGroup>
              <ListRow
                className="stg-row"
                hint={text(
                  'detectHint',
                  'AlverChat briefly turns on the camera and microphone so their names can be listed below.',
                )}
                icon="search"
                title={text('detectDevices', 'Choose camera and microphone')}
                trailing={
                  <Button loading={probing} onClick={detectDevices} size="sm" variant="secondary">
                    {text('detectButton', 'Detect devices')}
                  </Button>
                }
              />
              {row(
                'microphone',
                'mic',
                text('microphone', 'Microphone'),
                selectFor(text('microphone', 'Microphone'), settings.microphone, micOptions, (next) =>
                  update({ microphone: next }),
                ),
              )}
              {row(
                'speaker',
                'volume',
                text('speaker', 'Speaker'),
                selectFor(text('speaker', 'Speaker'), settings.speaker, speakerOptions, (next) =>
                  update({ speaker: next }),
                ),
              )}
              {row(
                'camera',
                'video',
                text('camera', 'Camera'),
                selectFor(text('camera', 'Camera'), settings.camera, cameraOptions, (next) =>
                  update({ camera: next }),
                ),
              )}
              {row(
                'noise',
                'zap',
                text('noise', 'Noise suppression'),
                toggleFor(text('noise', 'Noise suppression'), settings.noiseSuppression, (next) =>
                  update({ noiseSuppression: next }),
                ),
                text('noiseHint', 'Reduce background noise during calls.'),
              )}
              {row(
                'echo',
                'refreshCw',
                text('echo', 'Echo cancellation'),
                toggleFor(text('echo', 'Echo cancellation'), settings.echoCancellation, (next) =>
                  update({ echoCancellation: next }),
                ),
                text('echoHint', 'Prevent echo and audio feedback.'),
              )}
              <ListRow
                className="stg-row"
                hint={text('ringtoneHint', 'Sound played when someone calls you.')}
                icon="audioLines"
                title={text('ringtone', 'Ringtone')}
                trailing={
                  <span className="stg-pair">
                    <Button icon="play" onClick={() => playTone('ringtone')} size="sm" variant="secondary">
                      {text('play', 'Play')}
                    </Button>
                    {selectFor(text('ringtone', 'Ringtone'), settings.ringtone, ringtoneOptions, (next) =>
                      update({ ringtone: next }),
                    )}
                  </span>
                }
              />
              <ListRow
                className="stg-row"
                hint={text('testSetupHint', 'Check your microphone, camera and speakers right now.')}
                icon="gauge"
                title={text('testSetup', 'Test your setup')}
                trailing={
                  <Button loading={testingSetup} onClick={() => void testSetup()} size="sm" variant="primary">
                    {text('testSetupButton', 'Run Test')}
                  </Button>
                }
              />
            </ListGroup>
            {setupResult ? (
              <Banner
                message={setupResult.lines.join(' ')}
                tone={setupResult.ok ? 'success' : 'danger'}
                title={setupResult.ok ? text('testOkTitle', 'Everything looks good') : text('testFailTitle', 'The test did not pass')}
                action={
                  setupResult.ok ? null : (
                    <Button
                      onClick={() => {
                        void bridge.openPermissionSettings('microphone').catch(fail);
                      }}
                      size="sm"
                      variant="secondary"
                    >
                      {text('openPermissions', 'Open Settings')}
                    </Button>
                  )
                }
              />
            ) : null}
            {micPermission?.state === 'denied' ? (
              <Banner
                message={micPermission.hint}
                title={text('micBlocked', 'Your system is blocking the microphone')}
                tone="warning"
                action={
                  <Button
                    onClick={() => {
                      void bridge.openPermissionSettings('microphone').catch(fail);
                    }}
                    size="sm"
                    variant="secondary"
                  >
                    {text('openPermissions', 'Open Settings')}
                  </Button>
                }
              />
            ) : null}
            <p className="stg-note">
              {namedDevices(devices).length === 0
                ? text(
                    'devicesHidden',
                    'Names stay hidden until you allow media access — press Detect devices above.',
                  )
                : text('deviceCount', '{{count}} devices detected on this machine.', { count: devices.length })}
            </p>
          </div>
        ) : null}

        {category === 'privacy' ? (
          <div className="stg-stack">
            <ListGroup>
              {row(
                'lanEncryption',
                'lock',
                text('lanEncryption', 'LAN encryption'),
                // A status, not a switch. Nothing in the transport reads a stored preference
                // for this — what encrypts a session is the passphrase challenge — so a toggle
                // here could only ever say something the wire then contradicted.
                passphraseSet ? (
                  <Badge tone="success">{text('lanEncryptionOn', 'Encrypted')}</Badge>
                ) : (
                  <Badge tone="neutral">{text('lanEncryptionOff', 'Not encrypted')}</Badge>
                ),
                text(
                  'lanEncryptionHint',
                  'This follows the network passphrase below — a saved passphrase is what encrypts the traffic.',
                ),
              )}
              <ListRow
                className="stg-row"
                hint={
                  passphraseSet
                    ? text('passphraseSet', 'A passphrase is saved on this device.')
                    : text('passphraseNone', 'No passphrase is set yet.')
                }
                icon="shield"
                title={text('passphrase', 'LAN passphrase')}
                trailing={
                  <span className="stg-pair stg-pair--wide">
                    <Input
                      aria-label={text('passphrase', 'LAN passphrase')}
                      className="stg-input stg-input--pass"
                      onChange={(event) => setPassphrase(event.target.value)}
                      onTrailingClick={() => setShowPassphrase((current) => !current)}
                      placeholder={text('passphrasePlaceholder', 'Enter a passphrase')}
                      trailingIcon={showPassphrase ? 'eyeOff' : 'eye'}
                      trailingLabel={
                        showPassphrase
                          ? text('hidePassphrase', 'Hide passphrase')
                          : text('showPassphrase', 'Show passphrase')
                      }
                      type={showPassphrase ? 'text' : 'password'}
                      value={passphrase}
                    />
                    <Button onClick={savePassphrase} size="sm" variant="primary">
                      {text('savePassphrase', 'Save')}
                    </Button>
                  </span>
                }
              />
              {row(
                'whoSeesStatus',
                'eye',
                text('whoSeesStatus', 'Who can see my status'),
                selectFor(text('whoSeesStatus', 'Who can see my status'), settings.whoSeesStatus, statusOptions, (next) =>
                  update({ whoSeesStatus: next }),
                ),
              )}
              {row(
                'visible',
                'wifi',
                text('visible', 'Visible on network'),
                toggleFor(text('visible', 'Visible on network'), settings.visibleOnNetwork, (next) =>
                  update({ visibleOnNetwork: next }),
                ),
                text('visibleHint', 'Allow others to find and chat with me.'),
              )}
              {row(
                'announcements',
                'megaphone',
                text('announcements', 'Announcements from admins only'),
                toggleFor(
                  text('announcements', 'Announcements from admins only'),
                  settings.announcementsFromAdminsOnly,
                  (next) => update({ announcementsFromAdminsOnly: next }),
                ),
                text('announcementsHint', 'Restrict the network-wide banner to group admins.'),
              )}
              {row(
                'adapter',
                'network',
                text('adapter', 'Network adapter'),
                selectFor(text('adapter', 'Network adapter'), settings.networkAdapter, networkOptions, (next) =>
                  update({ networkAdapter: next }),
                ),
                text('adapterHint', 'Which interface AlverChat listens on.'),
              )}
            </ListGroup>

            <ListGroup section={text('diagnostics', 'Connection diagnostics')}>
              <ListRow
                className="stg-row"
                hint={text('diagnosticsHint', 'Check listening port, peers and adapters.')}
                icon="activity"
                title={text('diagnosticsTitle', 'Connection diagnostics')}
                trailing={
                  <Button loading={testingNetwork} onClick={runTest} size="sm" variant="primary">
                    {text('runTest', 'Run Test')}
                  </Button>
                }
              />
              {report ? (
                <div className="stg-report">
                  <div className="stg-report-head">
                    <Badge tone={report.listening ? 'success' : 'danger'}>
                      {report.listening
                        ? text('reportListening', 'Listening')
                        : text('reportSilent', 'Not listening')}
                    </Badge>
                    <span className="stg-report-ports">
                      {text('reportPort', 'Port {{port}}', { port: report.port })} ·{' '}
                      {text('reportDiscovery', 'Discovery {{port}}', { port: report.discoveryPort })}
                      {' · '}
                      {report.discoveryActive
                        ? text('reportDiscoveryLive', 'fallback listening')
                        : text('reportDiscoveryIdle', 'fallback not listening')}
                    </span>
                  </div>
                  <dl className="stg-report-grid">
                    <div>
                      <dt>{text('reportPeers', 'Peers found')}</dt>
                      <dd>{report.peers}</dd>
                    </div>
                    <div>
                      <dt>{text('reportOnline', 'Online peers')}</dt>
                      <dd>{report.onlinePeers}</dd>
                    </div>
                    <div>
                      <dt>{text('reportAdapters', 'Adapters')}</dt>
                      <dd>{report.adapters.length}</dd>
                    </div>
                  </dl>
                  <ul className="stg-report-list">
                    {report.adapters.length > 0 ? (
                      report.adapters.map((adapter) => (
                        <li key={adapter}>
                          <Icon name="network" size={14} />
                          {adapter}
                        </li>
                      ))
                    ) : (
                      <li className="stg-report-empty">{text('reportNoAdapters', 'No adapters reported.')}</li>
                    )}
                  </ul>
                  {report.warnings.length > 0 ? (
                    <div className="stg-report-warnings">
                      {report.warnings.map((warning) => (
                        <Banner key={warning} message={warning} tone="warning" />
                      ))}
                    </div>
                  ) : (
                    <p className="stg-note">{text('reportClean', 'No problems found on this network.')}</p>
                  )}
                </div>
              ) : (
                <p className="stg-note stg-report-empty">{text('reportIdle', 'No test has been run yet.')}</p>
              )}
            </ListGroup>

            <ListGroup section={text('firewall', 'Firewall help')}>
              <ListRow
                className="stg-row"
                hint={text('firewallHint', 'Get the steps for your operating system.')}
                icon="shieldCheck"
                title={text('firewallTitle', 'Firewall help')}
                trailing={
                  <Button icon="externalLink" onClick={openGuide} size="sm" variant="secondary">
                    {text('openGuide', 'Open Guide')}
                  </Button>
                }
              />
              {guideSteps.length > 0 ? (
                <ol className="stg-steps">
                  {guideSteps.map((step, index) => (
                    <li key={step}>
                      <span className="stg-step-number">{index + 1}</span>
                      <span>{step}</span>
                    </li>
                  ))}
                </ol>
              ) : (
                <p className="stg-note">{text('guideEmpty', 'No firewall steps are available on this platform.')}</p>
              )}
            </ListGroup>
          </div>
        ) : null}

        {category === 'about' ? (
          <div className="stg-stack">
            <div className="stg-about">
              <span className="stg-about-mark">
                <Icon name="messageCircle" size={26} filled />
              </span>
              <b className="stg-about-name">{APP_NAME}</b>
              <span className="stg-about-version">
                {text('versionLine', 'Version {{version}}', { version: APP_VERSION })}
              </span>
              <Badge tone={updateState === 'available' ? 'warning' : 'success'}>
                {updateState === 'available'
                  ? text('updateAvailable', 'Update available')
                  : text('upToDateBadge', 'Up to date')}
              </Badge>
            </div>

            <ListGroup>
              <ListRow
                chevron
                className="stg-row"
                hint={text('checkUpdatesHint', 'Ask the update server for a newer build.')}
                icon="refreshCw"
                onClick={() => void checkUpdates()}
                title={text('checkUpdates', 'Check for updates')}
              />
              {pendingUpdate ? (
                <div className="stg-about-actions">
                  <Button icon="download" loading={busy === 'update'} onClick={() => void installUpdate()} variant="primary">
                    {text('installUpdate', 'Install {{version}}', { version: pendingUpdate.version })}
                  </Button>
                </div>
              ) : null}
              {identity ? (
                <ListRow
                  className="stg-row"
                  hint={identity.fingerprint}
                  icon="badgeCheck"
                  title={text('identity', 'Your network fingerprint')}
                  trailing={<span className="stg-mono">{identity.peerId}</span>}
                />
              ) : null}
            </ListGroup>

            <ListGroup section={text('backupExport', 'Backup & Export')}>
              <ListRow
                className="stg-row"
                hint={text('backupHint', 'Create a backup of your chats, settings and groups.')}
                icon="archive"
                title={text('backupTitle', 'Backup & Export')}
              />
              <div className="stg-about-actions">
                <Button icon="archive" onClick={() => setBackupDialog(true)} variant="secondary">
                  {text('createBackup', 'Create Backup')}
                </Button>
                <Button icon="upload" loading={busy === 'export'} onClick={exportNow} variant="secondary">
                  {text('exportData', 'Export Data')}
                </Button>
              </div>
              <ListRow
                chevron
                className="stg-row"
                hint={text('restoreHint', 'Replace this device’s data with a backup file.')}
                icon="save"
                onClick={() => setRestoreDialog(true)}
                title={text('restore', 'Restore from Backup')}
              />
            </ListGroup>

            <ListGroup section={`${text('backupList', 'Backups')} (${backups.length})`}>
              {backups.length === 0 ? (
                <EmptyState
                  compact
                  icon="archive"
                  message={text('backupEmpty', 'Backups you create are listed here with their size and date.')}
                  title={text('backupEmptyTitle', 'No backups yet')}
                />
              ) : (
                backups.map((backup) => (
                  <ListRow
                    className="stg-row"
                    hint={`${formatFileSize(backup.bytes)} · ${dateLabel(backup.createdMs)}`}
                    icon="archive"
                    key={backup.path}
                    title={fileNameFor(backup.path)}
                    trailing={
                      <Button
                        icon="folder"
                        onClick={() => {
                          void bridge.revealPath(backup.path).catch(fail);
                        }}
                        size="sm"
                        variant="ghost"
                      >
                        {text('reveal', 'Show')}
                      </Button>
                    }
                  />
                ))
              )}
              {exportResult ? (
                <ListRow
                  className="stg-row"
                  hint={text('exportCount', '{{count}} messages exported.', { count: exportResult.messages })}
                  icon="download"
                  title={fileNameFor(exportResult.path)}
                  trailing={
                    <Button
                      icon="folder"
                      onClick={() => {
                        void bridge.revealPath(exportResult.path).catch(fail);
                      }}
                      size="sm"
                      variant="ghost"
                    >
                      {text('reveal', 'Show')}
                    </Button>
                  }
                />
              ) : null}
            </ListGroup>
          </div>
        ) : null}
      </div>

      {/* --- dialogs ---------------------------------------------------- */}
      <Dialog
        description={text(
          'clearHistoryWarning',
          'This deletes every local message and media file on this device. It cannot be undone.',
        )}
        footer={
          <>
            <Button onClick={() => setClearDialog(false)} variant="secondary">
              {text('cancel', 'Cancel')}
            </Button>
            <Button icon="trash2" onClick={() => void clearHistory()} variant="danger">
              {text('clearHistoryButton', 'Clear History')}
            </Button>
          </>
        }
        onOpenChange={setClearDialog}
        open={clearDialog}
        size="sm"
        title={text('clearHistoryTitle', 'Clear chat history?')}
      />

      <Dialog
        description={text('backupDescription', 'Choose a passphrase to encrypt the backup file.')}
        footer={
          <>
            <Button onClick={() => setBackupDialog(false)} variant="secondary">
              {text('cancel', 'Cancel')}
            </Button>
            <Button icon="archive" loading={busy === 'backup'} onClick={createBackupNow} variant="primary">
              {text('createBackup', 'Create Backup')}
            </Button>
          </>
        }
        onOpenChange={setBackupDialog}
        open={backupDialog}
        title={text('backupTitleDialog', 'Create a backup')}
      >
        <Input
          aria-label={text('backupPassphrase', 'Backup passphrase')}
          hint={text(
            'backupPassphraseHint',
            'You need it to restore this file, which also holds this device’s keys — keep it private.',
          )}
          label={text('backupPassphrase', 'Backup passphrase')}
          onChange={(event) => setBackupPassphrase(event.target.value)}
          placeholder={text('backupPassphrasePlaceholder', 'At least {{min}} characters', {
            min: BACKUP_PASSPHRASE_MIN_LEN,
          })}
          type="password"
          value={backupPassphrase}
        />
      </Dialog>

      <Dialog
        description={text('restoreDescription', 'Pick a backup below and enter its passphrase.')}
        footer={
          <>
            <Button onClick={() => setRestoreDialog(false)} variant="secondary">
              {text('cancel', 'Cancel')}
            </Button>
            <Button icon="save" loading={busy === 'restore'} onClick={restoreNow} variant="primary">
              {text('restoreButton', 'Restore')}
            </Button>
          </>
        }
        onOpenChange={setRestoreDialog}
        open={restoreDialog}
        title={text('restoreTitle', 'Restore from backup')}
      >
        <div className="stg-restore-list">
          {backups.length === 0 ? (
            <p className="stg-note">{text('restoreEmpty', 'No backups were found on this device.')}</p>
          ) : (
            backups.map((backup) => (
              <ListRow
                aria-pressed={restorePath === backup.path}
                className={
                  restorePath === backup.path
                    ? 'stg-restore-item stg-restore-item--on'
                    : 'stg-restore-item'
                }
                hint={`${formatFileSize(backup.bytes)} · ${dateLabel(backup.createdMs)}`}
                icon="archive"
                key={backup.path}
                leadingTone="plain"
                onClick={() => setRestorePath(backup.path)}
                title={fileNameFor(backup.path)}
                trailing={
                  restorePath === backup.path ? <Icon name="check" size={15} /> : undefined
                }
              />
            ))
          )}
        </div>
        <Input
          aria-label={text('restorePassphrase', 'Backup passphrase')}
          className="stg-restore-pass"
          label={text('restorePassphrase', 'Backup passphrase')}
          onChange={(event) => setRestorePassphrase(event.target.value)}
          type="password"
          value={restorePassphrase}
        />
      </Dialog>

      <Dialog
        description={text('mutedDescription', 'Turn a chat off so it never alerts you on this device.')}
        footer={
          <Button onClick={() => setMutedDialog(false)} variant="secondary">
            {text('done', 'Done')}
          </Button>
        }
        onOpenChange={setMutedDialog}
        open={mutedDialog}
        title={text('manageMuted', 'Manage muted chats')}
      >
        <div className="stg-muted-dialog">
          {peers.length === 0 ? (
            <p className="stg-note">{text('mutedNoPeers', 'No conversations found on this device.')}</p>
          ) : (
            peers.map((peer) => (
              <ListRow
                className="stg-row"
                hint={statusLineFor(peer)}
                key={peer.id}
                leading={
                  <Avatar
                    color={paletteColorFor(peer.color)}
                    group={peer.group}
                    initials={peer.initials}
                    name={peer.name}
                    size={24}
                  />
                }
                title={peer.name}
                trailing={
                  <Toggle
                    checked={Boolean(peer.muted)}
                    label={`${text('muteChat', 'Mute')} ${peer.name}`}
                    onChange={(next) => toggleChatMute(peer.id, next)}
                  />
                }
              />
            ))
          )}
        </div>
      </Dialog>
    </div>
  );
}

export { SettingsScreen };
