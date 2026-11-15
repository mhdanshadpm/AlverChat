import { useCallback, useEffect, useMemo, useRef, useState, type CSSProperties, type KeyboardEvent, type ReactNode } from 'react';
import {
  AVATAR_PALETTE,
  Avatar,
  Badge,
  Button,
  Dialog,
  Icon,
  IconButton,
  Input,
  ListGroup,
  ListRow,
  Skeleton,
  cx,
  initialsFor,
  type AvatarPaletteColor,
} from '../components';
import { ns, registerNamespace } from '../i18n';
import onboardingCatalog from '../i18n/en/onboarding';
import type { Peer, PermissionStatus } from '../../../bridge/src';
import type { ScreenProps } from './shared';
import { currentPlatform, fallbackFirewallSteps, platformLabel, type AlverPlatform } from './SystemStates';
import './OnboardingScreen.css';

registerNamespace('onboarding', onboardingCatalog);
const text = ns('onboarding');

/** Discovery re-polls on this cadence while the people step is on screen. */
const PEER_POLL_MS = 2000;

/* ==================================================================== *
 * Step model
 * ==================================================================== */

type StepId = 'welcome' | 'profile' | 'permissions' | 'people' | 'done';

/** Exported so the key guard can resolve each rail label against `onboarding`. */
export const STEPS: readonly { id: StepId; label: string; labelKey: string }[] = [
  { id: 'welcome', label: 'Welcome', labelKey: 'stepWelcome' },
  { id: 'profile', label: 'Profile', labelKey: 'stepProfile' },
  { id: 'permissions', label: 'Permissions', labelKey: 'stepPermissions' },
  { id: 'people', label: 'People', labelKey: 'stepPeople' },
  { id: 'done', label: 'All set', labelKey: 'stepDone' },
];

const LAST_STEP = STEPS.length - 1;

function stepLabel(index: number): string {
  const item = STEPS[index];
  return text(item.labelKey, item.label);
}

/* ==================================================================== *
 * Permissions
 * ==================================================================== */

/**
 * The OS asks for these on the user's behalf, so the list is what the native
 * shell can actually report — Windows owns the firewall prompt, macOS does not.
 */
const PERMISSION_KINDS: Record<AlverPlatform, readonly string[]> = {
  windows: ['local-network', 'firewall', 'camera', 'microphone'],
  macos: ['local-network', 'camera', 'microphone'],
  other: ['local-network', 'camera', 'microphone'],
};

type PermissionMeta = { icon: 'network' | 'shield' | 'camera' | 'mic'; label: string; hint: string };

function permissionMeta(kind: string): PermissionMeta {
  if (kind === 'firewall') {
    return {
      icon: 'shield',
      label: text('permFirewall', 'Firewall access'),
      hint: text('permFirewallHint', 'Lets other devices on this network reach AlverChat.'),
    };
  }
  if (kind === 'camera') {
    return {
      icon: 'camera',
      label: text('permCamera', 'Camera'),
      hint: text('permCameraHint', 'Needed for video calls. You can grant it later.'),
    };
  }
  if (kind === 'microphone') {
    return {
      icon: 'mic',
      label: text('permMic', 'Microphone'),
      hint: text('permMicHint', 'Needed for voice calls. You can grant it later.'),
    };
  }
  return {
    icon: 'network',
    label: text('permNetwork', 'Local network discovery'),
    hint: text('permNetworkHint', 'Finds AlverChat on the same Wi-Fi or Ethernet as this computer.'),
  };
}

type PermissionState = PermissionStatus['state'];

function stateLabel(state: PermissionState): string {
  if (state === 'granted') return text('stateGranted', 'Allowed');
  if (state === 'denied') return text('stateBlocked', 'Blocked');
  return text('stateUnknown', 'Needs check');
}

const STATE_TONE: Record<PermissionState, 'success' | 'danger' | 'neutral'> = {
  granted: 'success',
  denied: 'danger',
  unknown: 'neutral',
};

const PERMISSION_FALLBACK: PermissionStatus = {
  kind: '',
  label: '',
  hint: '',
  state: 'unknown',
  deepLink: '',
};

/* ==================================================================== *
 * Screen
 * ==================================================================== */

export function OnboardingScreen({ bridge, go, toast, settings, patchSettings }: ScreenProps) {
  const platform = useMemo(() => currentPlatform(), []);
  const kinds = PERMISSION_KINDS[platform];

  const [step, setStep] = useState(0);
  const [furthest, setFurthest] = useState(0);

  /* --- step 2: profile ------------------------------------------------ */
  const [displayName, setDisplayName] = useState(settings.displayName);
  const [avatarColor, setAvatarColor] = useState<AvatarPaletteColor>(paletteFrom(settings.avatarColor));
  const [nameError, setNameError] = useState('');
  const [photo, setPhoto] = useState<string | null>(null);
  const nameTouched = useRef(false);
  const fileInput = useRef<HTMLInputElement>(null);
  const nameInput = useRef<HTMLInputElement>(null);
  const rail = useRef<HTMLOListElement>(null);

  /* --- step 3: permissions -------------------------------------------- */
  const [permissions, setPermissions] = useState<Record<string, PermissionStatus>>({});
  const [permissionBusy, setPermissionBusy] = useState(false);
  const [guide, setGuide] = useState<readonly string[]>([]);
  const [guideOpen, setGuideOpen] = useState(false);

  /* --- step 4: people -------------------------------------------------- */
  const [discovered, setDiscovered] = useState<Peer[]>([]);
  const [discoveryBusy, setDiscoveryBusy] = useState(true);

  useEffect(() => {
    if (nameTouched.current || settings.displayName.trim().length === 0) return;
    setDisplayName(settings.displayName);
  }, [settings.displayName]);

  /* Permissions are only worth asking for when the card is on screen. */
  const loadPermissions = useCallback(async () => {
    setPermissionBusy(true);
    const entries = await Promise.all(
      kinds.map(async (kind): Promise<readonly [string, PermissionStatus]> => {
        try {
          return [kind, await bridge.permissionStatus(kind)] as const;
        } catch {
          return [kind, PERMISSION_FALLBACK] as const;
        }
      }),
    );
    setPermissions(Object.fromEntries(entries));
    setPermissionBusy(false);
  }, [bridge, kinds]);

  useEffect(() => {
    if (step !== 2) return;
    void loadPermissions();
  }, [loadPermissions, step]);

  /**
   * Discovery: one probe the moment the step opens, then every 2 s while the
   * window is visible. A hidden window asks for nothing and resumes at once on
   * `visibilitychange`, so a backgrounded setup never keeps hitting mDNS.
   */
  useEffect(() => {
    if (step !== 3) return;
    let alive = true;
    let timer: number | undefined;
    let fetching = false;

    const tick = async () => {
      if (fetching) return;
      fetching = true;
      try {
        const peers = await bridge.listPeers();
        if (alive) setDiscovered(peers.filter(peer => !peer.group));
      } catch {
        /* Keep the last list; the next tick retries. */
      } finally {
        fetching = false;
        if (alive) setDiscoveryBusy(false);
      }
    };

    const schedule = () => {
      if (timer !== undefined) window.clearTimeout(timer);
      timer =
        document.visibilityState === 'visible'
          ? window.setTimeout(async () => {
              timer = undefined;
              await tick();
              schedule();
            }, PEER_POLL_MS)
          : undefined;
    };

    setDiscoveryBusy(true);
    void tick();
    schedule();

    const onResume = () => {
      if (document.visibilityState !== 'visible') {
        if (timer !== undefined) window.clearTimeout(timer);
        timer = undefined;
        return;
      }
      void tick();
      schedule();
    };
    document.addEventListener('visibilitychange', onResume);

    return () => {
      alive = false;
      if (timer !== undefined) window.clearTimeout(timer);
      document.removeEventListener('visibilitychange', onResume);
    };
  }, [bridge, step]);

  /* The stored avatar, so re-opening setup shows the photo already picked. */
  useEffect(() => {
    let alive = true;
    void bridge
      .getAvatar()
      .then(dataUrl => {
        if (alive && dataUrl) setPhoto(dataUrl);
      })
      .catch(() => {
        /* No avatar store outside the desktop app. */
      });
    return () => {
      alive = false;
    };
  }, [bridge]);

  const grantedTotal = kinds.filter(kind => permissions[kind]?.state === 'granted').length;
  const onlineTotal = discovered.filter(peer => peer.online).length;

  const openFirewallGuide = useCallback(async () => {
    let steps: readonly string[] = [];
    try {
      steps = await bridge.openFirewallGuide();
    } catch {
      steps = [];
    }
    setGuide(steps.length > 0 ? steps : fallbackFirewallSteps(platform));
    setGuideOpen(true);
  }, [bridge, platform]);

  const openPermissionSettings = useCallback(
    async (kind: string) => {
      try {
        await bridge.openPermissionSettings(kind);
      } catch {
        toast(text('settingsUnavailable', 'System settings can only be opened from the desktop app.'), 'neutral');
      }
    },
    [bridge, toast],
  );

  /** The photo is stored as a data URL because that is what `set_avatar` takes. */
  const pickPhoto = useCallback(
    (file: File | undefined) => {
      if (!file) return;
      if (!file.type.startsWith('image/')) {
        toast(text('photoNotImage', 'Pick an image file.'), 'danger');
        return;
      }
      const reader = new FileReader();
      reader.onload = () => {
        const dataUrl = typeof reader.result === 'string' ? reader.result : null;
        if (!dataUrl) {
          toast(text('photoFailed', 'That photo could not be read.'), 'danger');
          return;
        }
        setPhoto(dataUrl);
        void bridge.setAvatar(dataUrl).catch(() => {
          toast(text('photoStoreFailed', 'The photo previews here but is saved on the profile screen.'), 'neutral');
        });
      };
      reader.onerror = () => toast(text('photoFailed', 'That photo could not be read.'), 'danger');
      reader.readAsDataURL(file);
    },
    [bridge, toast],
  );

  const removePhoto = useCallback(() => {
    setPhoto(null);
    void bridge.setAvatar(null).catch(() => {
      /* Nothing persisted, so nothing to undo. */
    });
  }, [bridge]);

  const saveProfile = useCallback(() => {
    const name = displayName.trim();
    if (name.length === 0) {
      setNameError(text('nameRequired', 'Add a display name so people can recognise you.'));
      return false;
    }
    patchSettings({ displayName: name, avatarColor });
    setDisplayName(name);
    setNameError('');
    return true;
  }, [avatarColor, displayName, patchSettings]);

  const advance = useCallback(() => {
    if (step === 1 && !saveProfile()) return;
    if (step === LAST_STEP) {
      if (settings.displayName.trim().length === 0) patchSettings({ displayName: displayName.trim() });
      toast(text('finishedToast', 'You are ready to chat on this network.'), 'success');
      go('chats');
      return;
    }
    const next = Math.min(LAST_STEP, step + 1);
    setStep(next);
    setFurthest(current => Math.max(current, next));
  }, [displayName, go, patchSettings, saveProfile, settings.displayName, step, toast]);

  const back = useCallback(() => setStep(current => Math.max(0, current - 1)), []);

  /* ⌘/Ctrl+Enter continues, Esc goes back — the window has no title-bar chrome. */
  const onKeyDown = useCallback(
    (event: KeyboardEvent<HTMLDivElement>) => {
      if (event.key === 'Enter' && (event.metaKey || event.ctrlKey)) {
        event.preventDefault();
        advance();
        return;
      }
      if (event.key === 'Escape' && step > 0) {
        event.preventDefault();
        back();
      }
    },
    [advance, back, step],
  );

  /* Arrow keys walk the progress rail. */
  const onRailKeyDown = useCallback((event: KeyboardEvent<HTMLOListElement>) => {
    if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return;
    const buttons = Array.from(rail.current?.querySelectorAll<HTMLButtonElement>('button:not(:disabled)') ?? []);
    if (buttons.length === 0) return;
    event.preventDefault();
    const at = buttons.indexOf(document.activeElement as HTMLButtonElement);
    const delta = event.key === 'ArrowDown' ? 1 : -1;
    const nextAt = at < 0 ? 0 : (at + delta + buttons.length) % buttons.length;
    buttons[nextAt]?.focus();
  }, []);

  useEffect(() => {
    if (step === 1) nameInput.current?.focus();
  }, [step]);

  const previewName = displayName.trim().length > 0 ? displayName.trim() : text('youPlaceholder', 'Your name');

  return (
    <div className="ob">
      <div className="ob-window" onKeyDown={onKeyDown} role="group">
        <p className="ob-window__label">{text('windowLabel', 'AlverChat setup')} · 720 × 520</p>

        <h1 className="ac-sr-only">{text('setupTitle', 'AlverChat setup')}</h1>
        <aside aria-label={text('setupProgress', 'Setup progress')} className="ob-rail">
          <span className="ob-brand">
            <Icon filled name="messageCircle" size={17} />
            <b>{text('brand', 'AlverChat')}</b>
          </span>
          <span className="ob-rail__os">
            <Icon name="monitor" size={13} />
            {text('runningOn', 'Running on {{os}}', { os: platformLabel(platform) })}
          </span>
          <ol className="ob-steps" onKeyDown={onRailKeyDown} ref={rail}>
            {STEPS.map((item, index) => {
              const done = index < step || index < furthest;
              const current = index === step;
              return (
                <li key={item.id}>
                  <button
                    aria-current={current ? 'step' : undefined}
                    className={cx('ob-steps__btn', current && 'is-current', done && 'is-done')}
                    disabled={index > furthest}
                    onClick={() => {
                      if (index > furthest) return;
                      setStep(index);
                    }}
                    type="button"
                  >
                    <span aria-hidden className="ob-steps__dot">
                      {index < step ? <Icon name="check" size={13} strokeWidth={3} /> : index + 1}
                    </span>
                    <span className="ob-steps__text">
                      <b>{stepLabel(index)}</b>
                      <small>{stepHint(item.id)}</small>
                    </span>
                    {index < furthest && !current ? <Icon className="ob-steps__seen" name="circle" size={9} /> : null}
                  </button>
                </li>
              );
            })}
          </ol>
          <span className="ob-rail__foot">
            {text('railFoot', 'You can change all of this later in Settings.')}
          </span>
        </aside>

        <div className="ob-body" key={step}>
          {step === 0 ? <WelcomeStep /> : null}

          {step === 1 ? (
            <section className="ob-step">
              <h2 className="ob-h">{text('profileTitle', 'Set up your profile')}</h2>
              <p className="ob-sub">
                {text('profileSub', 'Let others recognize you on this local network.')}
              </p>

              <div className="ob-card ob-profile">
                <Avatar
                  color={avatarColor}
                  initials={initialsFor(previewName)}
                  name={previewName}
                  size={56}
                  src={photo ?? undefined}
                />
                <span className="ob-profile__who">
                  <b>{previewName}</b>
                  <small>
                    {photo
                      ? text('profilePhotoOn', 'Photo avatar · visible to people on your network')
                      : text('profilePhotoOff', 'Initials avatar · change it any time')}
                  </small>
                </span>
                <Button icon="imagePlus" onClick={() => fileInput.current?.click()} size="sm" variant="secondary">
                  {photo ? text('photoChange', 'Change photo') : text('photoAdd', 'Add photo')}
                </Button>
                {photo ? (
                  <IconButton
                    iconSize={15}
                    label={text('photoRemove', 'Remove photo')}
                    name="trash"
                    onClick={() => removePhoto()}
                    size="sm"
                    variant="plain"
                  />
                ) : null}
              </div>

              <Input
                error={nameError}
                icon="user"
                label={text('nameLabel', 'Display name')}
                maxLength={40}
                onChange={event => {
                  nameTouched.current = true;
                  setDisplayName(event.target.value);
                  if (nameError) setNameError('');
                }}
                placeholder={text('namePlaceholder', 'Enter a display name')}
                ref={nameInput}
                showCount
                size="lg"
                value={displayName}
              />

              <p className="ob-label" id="ob-palette-label">
                {text('colorLabel', 'Avatar colour')}
              </p>
              <div aria-labelledby="ob-palette-label" className="ob-palette" role="radiogroup">
                {AVATAR_PALETTE.map(color => (
                  <button
                    aria-checked={color === avatarColor}
                    aria-label={text('swatchAria', '{{color}} avatar', { color })}
                    className="ob-swatch"
                    key={color}
                    onClick={() => setAvatarColor(color)}
                    role="radio"
                    style={{ '--ob-swatch-a': `var(--ac-av-${color}-a)`, '--ob-swatch-b': `var(--ac-av-${color}-b)` } as CSSProperties}
                    type="button"
                  >
                    {color === avatarColor ? <Icon name="check" size={14} strokeWidth={3} /> : null}
                  </button>
                ))}
              </div>

              {photo ? (
                <div className="ob-sizes">
                  <span className="ob-label">{text('sizesLabel', 'How your photo looks at each size')}</span>
                  <span className="ob-sizes__row">
                    {([56, 40, 24] as const).map(size => (
                      <span className="ob-sizes__item" key={size}>
                        <Avatar color={avatarColor} initials={initialsFor(previewName)} name={previewName} size={size} src={photo} />
                        <small>{size}px</small>
                      </span>
                    ))}
                  </span>
                </div>
              ) : null}

              <input
                accept="image/*"
                aria-label={text('photoInputAria', 'Choose a profile photo')}
                className="ob-file"
                onChange={event => pickPhoto(event.target.files?.[0])}
                ref={fileInput}
                tabIndex={-1}
                type="file"
              />
            </section>
          ) : null}

          {step === 2 ? (
            <section className="ob-step">
              <h2 className="ob-h">{text('permTitle', 'Permissions & network')}</h2>
              <p className="ob-sub">
                {text('permSub', 'AlverChat needs these to find peers and place calls on {{os}}.', {
                  os: platformLabel(platform),
                })}
              </p>

              {permissionBusy ? (
                <div className="ob-perms">
                  {kinds.map(kind => (
                    <div className="ob-perm ob-perm--busy" key={kind}>
                      <Skeleton variant="circle" />
                      <span className="ob-perm__lines">
                        <Skeleton variant="title" />
                        <Skeleton variant="text" />
                      </span>
                    </div>
                  ))}
                </div>
              ) : (
                <div className="ob-perms">
                  {kinds.map(kind => {
                    const status = permissions[kind] ?? PERMISSION_FALLBACK;
                    const meta = permissionMeta(kind);
                    const state = status.state;
                    return (
                      <article className="ob-perm" data-state={state} key={kind}>
                        <span className={cx('ob-perm__icon', `ob-perm__icon--${STATE_TONE[state]}`)}>
                          <Icon name={meta.icon} size={18} />
                        </span>
                        <span className="ob-perm__copy">
                          <b>{status.label.trim().length > 0 ? status.label : meta.label}</b>
                          <small>{status.hint.trim().length > 0 ? status.hint : meta.hint}</small>
                        </span>
                        <Badge tone={STATE_TONE[state]}>{stateLabel(state)}</Badge>
                        <Button
                          onClick={() => {
                            if (platform === 'windows') {
                              void openFirewallGuide();
                              return;
                            }
                            if (platform === 'macos') {
                              void openPermissionSettings(kind);
                              return;
                            }
                            void loadPermissions();
                          }}
                          size="sm"
                          variant="secondary"
                        >
                          {platform === 'windows'
                            ? text('permAllowFirewall', 'Allow in Firewall')
                            : platform === 'macos'
                              ? text('permOpenSettings', 'Open System Settings')
                              : text('permRecheck', 'Check again')}
                        </Button>
                      </article>
                    );
                  })}
                </div>
              )}

              <button className="ob-refresh" onClick={() => void loadPermissions()} type="button">
                <Icon name="refreshCw" size={13} />
                {text('permRefresh', 'Re-check permissions')}
              </button>

              <p className="ob-note">
                <Icon name="lock" size={14} />
                {text('permNote', 'Nothing leaves your local network. Messages stay end-to-end encrypted.')}
              </p>
            </section>
          ) : null}

          {step === 3 ? (
            <section className="ob-step">
              <h2 className="ob-h">{text('peopleTitle', 'People found on your network')}</h2>
              <p className="ob-sub">
                {discovered.length > 0
                  ? text('peopleFound', '{{total}} devices spotted · {{online}} online', {
                      total: discovered.length,
                      online: onlineTotal,
                    })
                  : text('peopleSearching', 'Still looking. Keep AlverChat open on your other devices.')}
              </p>

              {discovered.length === 0 ? (
                <div className="ob-scan" role="status">
                  <span aria-hidden className="ob-scan__radar">
                    <span className="ob-scan__ring" />
                    <span className="ob-scan__ring ob-scan__ring--late" />
                    <Icon name="scan" size={20} />
                  </span>
                  <b>{text('scanTitle', 'Scanning your local network…')}</b>
                  <small>{text('scanBody', 'New devices appear here automatically.')}</small>
                  {discoveryBusy ? null : (
                    <span className="ob-scan__tip">
                      <Icon name="info" size={13} />
                      {text('scanTip', 'Make sure the other device is on the same Wi-Fi and running AlverChat.')}
                    </span>
                  )}
                </div>
              ) : (
                <ListGroup className="ob-people">
                  {discovered.map(peer => (
                    <ListRow
                      key={peer.id}
                      leading={<Avatar color={paletteFrom(peer.color)} initials={peer.initials} name={peer.name} size={40} />}
                      hint={peer.online ? text('online', 'Online') : text('offline', 'Offline')}
                      title={peer.name}
                      trailing={peer.online ? <Badge tone="success">{text('ready', 'Ready')}</Badge> : null}
                    />
                  ))}
                </ListGroup>
              )}

              <p className="ob-note">
                <Icon name="usersRound" size={14} />
                {text('peopleNote', 'You can message anyone here the moment they appear.')}
              </p>
            </section>
          ) : null}

          {step === 4 ? (
            <section className="ob-step">
              <h2 className="ob-h">{text('doneTitle', 'You are all set')}</h2>
              <p className="ob-sub">{text('doneSub', 'AlverChat is ready on this device.')}</p>

              <div className="ob-summary">
                <span className="ob-summary__avatar">
                  <Avatar
                    color={avatarColor}
                    initials={initialsFor(previewName)}
                    name={previewName}
                    presence={onlineTotal > 0 ? 'online' : 'offline'}
                    size={56}
                    src={photo ?? undefined}
                  />
                  <b>{previewName}</b>
                  <small>{text('summaryVisible', 'Discoverable on this network')}</small>
                </span>
                <span className="ob-summary__stat">
                  <b>
                    {grantedTotal}/{kinds.length}
                  </b>
                  <small>{text('summaryPerms', 'permissions ready')}</small>
                </span>
                <span className="ob-summary__stat">
                  <b>{discovered.length}</b>
                  <small>{text('summaryPeople', 'people found')}</small>
                </span>
              </div>

              <ul className="ob-checks">
                <li>
                  <Icon name="checkCheck" size={15} />
                  {text('checkProfile', 'Profile saved on this device')}
                </li>
                <li>
                  <Icon name="checkCheck" size={15} />
                  {grantedTotal === kinds.length
                    ? text('checkPermsAll', 'All permissions granted')
                    : text('checkPermsSome', '{{total}} of {{size}} permissions granted', {
                        total: grantedTotal,
                        size: kinds.length,
                      })}
                </li>
                <li>
                  <Icon name={discovered.length > 0 ? 'checkCheck' : 'timer'} size={15} />
                  {discovered.length > 0
                    ? text('checkPeopleFound', 'Discovery is live · {{total}} peers', { total: discovered.length })
                    : text('checkPeopleWait', 'Discovery keeps running in the background')}
                </li>
              </ul>

              <p className="ob-note">
                <Icon name="shieldCheck" size={14} />
                {text('doneNote', 'Everything is stored locally. No account, no server, no phone home.')}
              </p>
            </section>
          ) : null}
        </div>

        <footer className="ob-foot">
          <Button disabled={step === 0} onClick={back} variant="ghost">
            {text('back', 'Back')}
          </Button>
          <span className="ob-foot__progress">
            {text('stepOf', 'Step {{current}} of {{total}}', { current: step + 1, total: STEPS.length })}
          </span>
          <Button icon={step === LAST_STEP ? 'check' : 'chevronRight'} onClick={advance} variant="primary">
            {step === 0
              ? text('getStarted', 'Get started')
              : step === LAST_STEP
                ? text('finish', 'Finish setup')
                : text('continue', 'Continue')}
          </Button>
        </footer>
      </div>

      <Dialog
        description={text('guideSub', 'AlverChat needs to be allowed through the {{os}} firewall to receive LAN messages.', {
          os: platformLabel(platform),
        })}
        footer={
          <Button onClick={() => setGuideOpen(false)} variant="primary">
            {text('guideDone', 'Got it')}
          </Button>
        }
        onOpenChange={setGuideOpen}
        open={guideOpen}
        size="md"
        title={text('guideTitle', 'Allow AlverChat through the firewall')}
      >
        <ol className="ob-guide">
          {guide.map(line => (
            <li key={line}>{line}</li>
          ))}
        </ol>
        <p className="ob-note">
          <Icon name="info" size={14} />
          {text('guideNote', 'You need administrator rights to change firewall rules.')}
        </p>
      </Dialog>
    </div>
  );
}

/* ==================================================================== *
 * Pieces
 * ==================================================================== */

function WelcomeStep(): ReactNode {
  return (
    <section className="ob-step ob-step--center">
      <span className="ob-welcome__art">
        <Icon filled name="messageCircle" size={40} />
        <Icon className="ob-welcome__spark" name="sparkles" size={16} />
      </span>
      <h2 className="ob-h">{text('welcomeTitle', 'Chat on your network.')}</h2>
      <p className="ob-sub">
        {text('welcomeSub', 'Discover peers and exchange end-to-end encrypted messages on your local network.')}
      </p>
      <ul className="ob-benefits">
        {([
          { icon: 'network', label: text('benefitLan', 'LAN discovery') },
          { icon: 'hardDrive', label: text('benefitLocal', 'Local data storage') },
          { icon: 'lock', label: text('benefitPrivate', 'Private identity') },
        ] as const).map(item => (
          <li key={item.label}>
            <Icon name={item.icon} size={15} />
            {item.label}
          </li>
        ))}
      </ul>
    </section>
  );
}

function stepHint(id: StepId): string {
  if (id === 'welcome') return text('hintWelcome', 'What AlverChat does');
  if (id === 'profile') return text('hintProfile', 'Name and avatar');
  if (id === 'permissions') return text('hintPermissions', 'Access on this OS');
  if (id === 'people') return text('hintPeople', 'Live discovery');
  return text('hintDone', 'Review and finish');
}

/** Settings stores a free string; the picker only accepts the 8 palette slots. */
function paletteFrom(color: string): AvatarPaletteColor {
  return (AVATAR_PALETTE as readonly string[]).includes(color) ? (color as AvatarPaletteColor) : 'violet';
}

export default OnboardingScreen;
