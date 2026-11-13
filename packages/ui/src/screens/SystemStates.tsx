import { useCallback, useEffect, useMemo, useState, type ReactNode } from 'react';
import {
  Badge,
  Banner,
  Button,
  EmptyState,
  Icon,
  IconButton,
  ListGroup,
  ListRow,
  ToastViewport,
  type IconName,
  type ToastView,
} from '../components';
import { ns } from '../i18n';
import type { ConnectionReport } from '../../../bridge/src';
import { ScreenHeader, type ScreenProps } from './shared';
import './SystemStates.css';

const text = ns('system');

/* ==================================================================== *
 * Platform
 *
 * The boards draw a different permission and firewall flow for Windows
 * and macOS, and the bridge only exposes one `openFirewallGuide()`, so
 * every per-OS branch in these widgets keys off this one helper.
 * ==================================================================== */

export type AlverPlatform = 'windows' | 'macos' | 'other';

/**
 * Review seam: `?platform=windows|macos` forces one OS so the per-OS permission
 * and firewall flows can be screenshotted from a single machine. Without it the
 * webview user agent decides, exactly as at runtime.
 */
function forcedPlatform(): AlverPlatform | null {
  try {
    const raw = new URLSearchParams(window.location.search).get('platform');
    return raw === 'windows' || raw === 'macos' || raw === 'other' ? raw : null;
  } catch {
    return null;
  }
}

/** Windows vs macOS from the webview UA string (the app never asks the OS). */
export function currentPlatform(): AlverPlatform {
  const forced = forcedPlatform();
  if (forced) return forced;
  const ua = typeof navigator === 'undefined' ? '' : navigator.userAgent;
  if (/Windows/i.test(ua)) return 'windows';
  if (/Mac|iPhone|iPad|iPod/i.test(ua)) return 'macos';
  return 'other';
}

export function platformLabel(platform = currentPlatform()): string {
  if (platform === 'windows') return text('osWindows', 'Windows');
  if (platform === 'macos') return text('osMacos', 'macOS');
  return text('osOther', 'This device');
}

/* ==================================================================== *
 * Connection cards (spec 7.12, board 2 "Connection & Network States")
 * ==================================================================== */

export type ConnectionState = 'no-network' | 'connected' | 'reconnecting' | 'peer-offline' | 'firewall';

export type ConnectionCardsProps = {
  /** The last `bridge.runConnectionTest()` result, or null while it is in flight. */
  report: ConnectionReport | null;
  onRunTest?: () => void;
  /**
   * `bridge.openFirewallGuide`. Resolved when the firewall card is expanded, so
   * the step list always comes from the OS instead of being guessed here.
   */
  onOpenGuide?: () => Promise<readonly string[]> | readonly string[];
  /** Name shown on the "Peer offline" card. */
  peerName?: string;
  /** Force one card, e.g. the gallery renders all five at once. */
  state?: ConnectionState;
  /** Render every card, stacked, exactly as the board shows them. */
  all?: boolean;
  className?: string;
};

/** Maps a `ConnectionReport` onto the single card the user should see. */
export function connectionStateFor(report: ConnectionReport | null): ConnectionState {
  if (!report) return 'reconnecting';
  const warnings = report.warnings.join(' ').toLowerCase();
  if (/firewall|blocked/.test(warnings)) return 'firewall';
  if (!report.listening || report.port === 0) return 'no-network';
  if (/reconnect|restore/.test(warnings)) return 'reconnecting';
  if (report.peers > 0 && report.onlinePeers === 0) return 'peer-offline';
  return 'connected';
}

const GUIDE_ICON: Record<ConnectionState, IconName> = {
  'no-network': 'wifi',
  connected: 'usersRound',
  reconnecting: 'refreshCw',
  'peer-offline': 'user',
  firewall: 'shield',
};

const GUIDE_TONE: Record<ConnectionState, 'danger' | 'success' | 'warning' | 'accent'> = {
  'no-network': 'danger',
  connected: 'success',
  reconnecting: 'accent',
  'peer-offline': 'warning',
  firewall: 'danger',
};

/** Fallback steps for the browser preview, where the bridge returns none. */
export function fallbackFirewallSteps(platform = currentPlatform()): string[] {
  if (platform === 'macos') {
    return [
      text('guideMac1', 'Open System Settings → Network & Internet → Firewall.'),
      text('guideMac2', 'Click Options and find AlverChat in the list.'),
      text('guideMac3', 'Set AlverChat to "allow incoming connections", then Allow Local Network in Privacy & Security.'),
    ];
  }
  return [
    text('guideWin1', 'Open Windows Security → Firewall & network protection.'),
    text('guideWin2', 'Click Allow an app through firewall.'),
    text('guideWin3', 'Find AlverChat in the list and allow it for Private networks (LAN).'),
  ];
}

export function ConnectionCard({
  state,
  report,
  peerName,
  steps,
  onRunTest,
  onOpenGuide,
  className,
}: {
  state: ConnectionState;
  report?: ConnectionReport | null;
  peerName?: string;
  steps?: readonly string[];
  onRunTest?: () => void;
  onOpenGuide?: () => Promise<readonly string[]> | readonly string[];
  className?: string;
}) {
  const [expanded, setExpanded] = useState(state !== 'firewall');
  const [guide, setGuide] = useState<readonly string[]>(steps ?? []);
  const [loadingGuide, setLoadingGuide] = useState(false);

  useEffect(() => {
    if (steps) setGuide(steps);
  }, [steps]);

  const showGuide = useCallback(async () => {
    const next = !expanded;
    setExpanded(next);
    if (!next || guide.length > 0) return;
    setLoadingGuide(true);
    try {
      const fetched = onOpenGuide ? await onOpenGuide() : [];
      setGuide(fetched.length > 0 ? fetched : fallbackFirewallSteps());
    } catch {
      setGuide(fallbackFirewallSteps());
    } finally {
      setLoadingGuide(false);
    }
  }, [expanded, guide.length, onOpenGuide]);

  const copy = useMemo(() => cardCopy(state, report ?? null, peerName), [state, report, peerName]);

  return (
    <div className={`ss-conn ss-conn--${GUIDE_TONE[state]}${className ? ` ${className}` : ''}`}>
      <span className={`ss-conn__icon ss-conn__icon--${GUIDE_TONE[state]}`}>
        <Icon
          name={GUIDE_ICON[state]}
          size={18}
          spin={state === 'reconnecting' ? true : undefined}
        />
      </span>
      <span className="ss-conn__body">
        <span className="ss-conn__title">{copy.title}</span>
        <span className="ss-conn__text">{copy.text}</span>
        {copy.detail ? <span className="ss-conn__detail">{copy.detail}</span> : null}
      </span>
      <span className="ss-conn__actions">{copy.action(onRunTest, state === 'firewall' ? showGuide : undefined, expanded)}</span>
      {state === 'firewall' && expanded ? (
        <div className="ss-conn__guide">
          <span className="ss-conn__guide-title">
            {text(
              'guideHeading',
              'How to allow in the firewall ({{os}})',
              { os: currentPlatform() === 'macos' ? 'macOS' : 'Windows' },
            )}
          </span>
          {loadingGuide ? (
            <span className="ss-conn__guide-loading">
              <Icon name="loader" size={14} spin />
              {text('guideLoading', 'Reading your firewall settings…')}
            </span>
          ) : null}
          <ol className="ss-conn__steps">
            {guide.map((step, index) => (
              <li className="ss-conn__step" key={`${step}-${index}`}>
                <span className="ss-conn__step-num">{index + 1}</span>
                <span>{step}</span>
              </li>
            ))}
          </ol>
        </div>
      ) : null}
    </div>
  );
}

type CardCopy = {
  title: string;
  text: string;
  detail: string;
  action: (onRunTest?: () => void, onGuide?: () => void, expanded?: boolean) => ReactNode;
};

function cardCopy(
  state: ConnectionState,
  report: ConnectionReport | null,
  peerName?: string,
): CardCopy {
  const online = report?.onlinePeers ?? 0;
  switch (state) {
    case 'no-network':
      return {
        title: text('connNoNetwork', 'No network'),
        text: text('connNoNetworkBody', "You're not connected to a network."),
        detail: text('connNoNetworkDetail', 'Connect to your office or local network (LAN) to find and chat with others.'),
        action: onRunTest => (
          <Button onClick={onRunTest} size="sm" variant="secondary">
            {text('connCheckNetwork', 'Check Network')}
          </Button>
        ),
      };
    case 'connected':
      return {
        title: text('connConnected', 'Connected to {{total}} peers', { total: online }),
        text: text('connConnectedBody', "You're connected to your LAN."),
        detail: text('connConnectedDetail', '{{total}} people are available on your network.', { total: online }),
        action: () => (
          <Badge tone="success">
            <Icon name="check" size={13} />
            {text('connConnectedPill', 'Connected')}
          </Badge>
        ),
      };
    case 'reconnecting':
      return {
        title: text('connReconnecting', 'Reconnecting…'),
        text: text('connReconnectingBody', 'Trying to restore your connection to the network…'),
        detail: '',
        action: () => (
          <Badge tone="warning">
            <Icon name="loader" size={13} spin />
            {text('connReconnectingPill', 'Reconnecting')}
          </Badge>
        ),
      };
    case 'peer-offline':
      return {
        title: text('connPeerOffline', 'Peer offline'),
        text: text('connPeerOfflineBody', '{{name}} is currently offline.', { name: peerName ?? text('connPeerFallback', 'That person') }),
        detail: text('connPeerOfflineDetail', 'Messages will be delivered when they come back.'),
        action: () => (
          <Button icon="messageCircle" size="sm" variant="secondary">
            {text('connMessage', 'Message')}
          </Button>
        ),
      };
    case 'firewall':
      return {
        title: text('connFirewall', 'Firewall blocked'),
        text: text('connFirewallBody', 'AlverChat is blocked by your firewall.'),
        detail: text('connFirewallDetail', 'Allow it to communicate on your network.'),
        action: (onRunTest, onGuide, expanded) => (
          <>
            <Button onClick={onGuide} size="sm" variant="danger">
              {expanded
                ? text('connHideSteps', 'Hide Steps')
                : text('connShowSteps', 'Show Steps')}
            </Button>
            <IconButton
              label={text('connRunTest', 'Run test')}
              name="refreshCw"
              onClick={onRunTest}
              size="sm"
              variant="raised"
            />
          </>
        ),
      };
  }
}

/** The stack the shell mounts under a screen header. */
export function ConnectionCards({
  report,
  onRunTest,
  onOpenGuide,
  peerName,
  state,
  all = false,
  className,
}: ConnectionCardsProps) {
  const active = state ?? connectionStateFor(report);
  const list: ConnectionState[] = all
    ? ['no-network', 'connected', 'reconnecting', 'peer-offline', 'firewall']
    : [active];
  return (
    <div className={`ss-conn-stack${className ? ` ${className}` : ''}`}>
      {list.map(item => (
        <ConnectionCard
          key={item}
          onOpenGuide={onOpenGuide}
          onRunTest={onRunTest}
          peerName={peerName}
          report={report}
          state={item}
        />
      ))}
    </div>
  );
}

/* ==================================================================== *
 * Empty states (spec 7.12, board 3 + 4)
 * ==================================================================== */

export type EmptyStateKind = 'chats' | 'results' | 'media' | 'pinned';

export type EmptyStatesProps = {
  kind?: EmptyStateKind;
  /** Query echoed back in the "no results" copy. */
  query?: string;
  /** Renders all four kinds side by side, as the board does. */
  all?: boolean;
  onAction?: () => void;
  className?: string;
};

export function NoChatsEmpty({ onAction }: { onAction?: () => void }) {
  return (
    <EmptyState
      actions={onAction ? <Button onClick={onAction}>{text('emptyFindPeople', 'Find People')}</Button> : undefined}
      icon="messageCircle"
      message={text('emptyChatsMessage', 'Start a conversation with someone on your network.')}
      title={text('emptyChatsTitle', 'No chats yet')}
    />
  );
}

export function NoResultsEmpty({ onAction, query }: { onAction?: () => void; query?: string }) {
  return (
    <EmptyState
      actions={onAction ? <Button onClick={onAction} variant="secondary">{text('emptyClearSearch', 'Clear Search')}</Button> : undefined}
      icon="searchX"
      message={text('emptyResultsMessage', 'We couldn’t find any matches for “{{query}}”.', { query: query ?? '' })}
      title={text('emptyResultsTitle', 'No results found')}
    />
  );
}

export function NoMediaEmpty() {
  return (
    <EmptyState
      icon="image"
      message={text('emptyMediaMessage', 'Photos, videos and files shared in this chat will appear here.')}
      title={text('emptyMediaTitle', 'No shared media')}
    />
  );
}

export function NoPinnedEmpty() {
  return (
    <EmptyState
      icon="pin"
      message={text('emptyPinnedMessage', 'Pin important messages to find them quickly later.')}
      title={text('emptyPinnedTitle', 'No pinned messages')}
    />
  );
}

/** One wrapper for whichever empty state a panel needs. */
export function EmptyStates({ kind = 'chats', query, all = false, onAction, className }: EmptyStatesProps) {
  if (all) {
    return (
      <div className={`ss-empty-grid${className ? ` ${className}` : ''}`}>
        <div className="ss-empty-card"><NoChatsEmpty onAction={onAction} /></div>
        <div className="ss-empty-card"><NoResultsEmpty onAction={onAction} query={query} /></div>
        <div className="ss-empty-card"><NoMediaEmpty /></div>
        <div className="ss-empty-card"><NoPinnedEmpty /></div>
      </div>
    );
  }
  if (kind === 'results') return <NoResultsEmpty onAction={onAction} query={query} />;
  if (kind === 'media') return <NoMediaEmpty />;
  if (kind === 'pinned') return <NoPinnedEmpty />;
  return <NoChatsEmpty onAction={onAction} />;
}

/* ==================================================================== *
 * LAN announcement (spec 7.12, board 4)
 * ==================================================================== */

export type Announcement = {
  id: string;
  text: string;
  from: string;
  at: string;
  acknowledgedBy: string[];
};

export type AnnouncementBannerProps = {
  announcement: Announcement;
  /** Sender-side ack callback wired to `announcement_ack`. */
  acknowledge?: () => void;
  /** Overrides the list on the announcement, e.g. live ack receipts. */
  acknowledgedBy?: readonly string[];
  /** Only the sender sees the "Acknowledged by (N)" list. */
  isSender?: boolean;
  /** Already acknowledged: the button becomes a quiet confirmation. */
  acknowledged?: boolean;
  /** Quiet provenance line — group, author, time — for live data. */
  meta?: ReactNode;
  onDismiss?: () => void;
  className?: string;
};

export function AnnouncementBanner({
  announcement,
  acknowledge,
  acknowledgedBy,
  isSender = false,
  acknowledged = false,
  meta,
  onDismiss,
  className,
}: AnnouncementBannerProps) {
  const [open, setOpen] = useState(false);
  const names = acknowledgedBy ?? announcement.acknowledgedBy;

  return (
    <div className={`ss-announce${className ? ` ${className}` : ''}`}>
      <Banner
        action={
          acknowledged ? (
            <Badge tone="success">
              <Icon name="check" size={13} />
              {text('acknowledged', 'Acknowledged')}
            </Badge>
          ) : acknowledge ? (
            <Button onClick={acknowledge} size="sm">
              {text('acknowledge', 'Acknowledge')}
            </Button>
          ) : undefined
        }
        icon="megaphone"
        inline
        message={announcement.text}
        title={text('announcementTitle', 'LAN Announcement')}
      />
      {meta ? <div className="ss-announce__meta">{meta}</div> : null}
      {isSender ? (
        <div className="ss-announce__ack">
          <button
            aria-controls={`announce-acks-${announcement.id}`}
            aria-expanded={open}
            className="ss-announce__ack-toggle"
            onClick={() => setOpen(value => !value)}
            type="button"
          >
            <Icon name="badgeCheck" size={15} />
            <span>
              {text('acknowledgedBy', 'Acknowledged by ({{total}})', { total: names.length })}
            </span>
            <Icon name={open ? 'chevronUp' : 'chevronDown'} size={15} />
          </button>
          {names.length > 0 ? (
            <span className="ss-announce__ack-names">
              {open
                ? names.join(text('nameSeparator', ', '))
                : `${names.slice(0, 3).join(text('nameSeparator', ', '))}${names.length > 3 ? '…' : ''}`}
            </span>
          ) : (
            <span className="ss-announce__ack-names ss-announce__ack-names--empty">
              {text('acknowledgedNone', 'Nobody has acknowledged this yet.')}
            </span>
          )}
          {open && names.length > 3 ? (
            <button className="ss-announce__view-all" onClick={onDismiss} type="button">
              {text('viewAll', 'View All')}
            </button>
          ) : null}
        </div>
      ) : null}
      {open ? (
        <ul className="ss-announce__list" id={`announce-acks-${announcement.id}`}>
          {names.map(name => (
            <li className="ss-announce__person" key={name}>
              <Icon name="check" size={13} />
              <span>{name}</span>
              <time>{announcement.at}</time>
            </li>
          ))}
        </ul>
      ) : null}
    </div>
  );
}

/* ==================================================================== *
 * Toast stack (spec 7.12, board 5)
 * ==================================================================== */

export type ToastStackProps = {
  /** Live toasts from the shell. Omit it (or pass []) with `demo` for the board set. */
  toasts?: readonly ToastView[];
  onDismiss?: (id: string) => void;
  /** Show the four showcase toasts as a static preview. */
  demo?: boolean;
  className?: string;
};

function demoToasts(): ToastView[] {
  return [
    {
      id: 'demo-sent',
      title: text('toastSent', 'Message sent'),
      message: text('toastSentBody', 'Delivered to {{name}}.', { name: 'Arjun Kumar' }),
      tone: 'neutral',
      icon: 'check',
      durationMs: 0,
    },
    {
      id: 'demo-failed',
      title: text('toastFailed', 'Message failed to send'),
      message: text('toastFailedBody', "Couldn't reach the recipient."),
      tone: 'danger',
      icon: 'alertCircle',
      action: { label: text('toastRetry', 'Retry'), onClick: () => {} },
      durationMs: 0,
    },
    {
      id: 'demo-uploaded',
      title: text('toastUploaded', 'File uploaded'),
      message: text('toastUploadedBody', '{{name}} ({{size}})', { name: 'report.pdf', size: '2.4 MB' }),
      tone: 'neutral',
      icon: 'info',
      durationMs: 0,
    },
    {
      id: 'demo-update',
      title: text('toastUpdate', 'Update available'),
      message: text('toastUpdateBody', '{{name}} {{version}} is ready to install.', { name: 'AlverChat', version: 'v2.5.0' }),
      tone: 'accent',
      icon: 'cloudDownload',
      action: { label: text('toastUpdateAction', 'Update'), onClick: () => {} },
      durationMs: 0,
    },
  ];
}

/** The viewport the shell mounts once per window. */
export function ToastStack({ toasts, onDismiss, demo = false, className }: ToastStackProps) {
  const [dismissed, setDismissed] = useState<string[]>([]);
  const [sample] = useState<ToastView[]>(demoToasts);
  const visible =
    !demo && toasts && toasts.length > 0 ? toasts : sample.filter(item => !dismissed.includes(item.id));
  const hide = useCallback(
    (id: string) => {
      if (onDismiss) onDismiss(id);
      else setDismissed(current => [...current, id]);
    },
    [onDismiss],
  );
  if (visible.length === 0) return null;
  return (
    <ToastViewport
      className={className}
      onDismiss={hide}
      toasts={visible}
    />
  );
}

/* ==================================================================== *
 * Preview screen — mounts at #/preview/systemstates
 * ==================================================================== */

export function SystemStates({ bridge, toast }: ScreenProps) {
  const [report, setReport] = useState<ConnectionReport | null>(null);
  const [showToasts, setShowToasts] = useState(false);
  const [announcement, setAnnouncement] = useState<Announcement>({
    id: 'ann-1',
    text: text('announcementSample', 'Office network maintenance today from 6:00 PM to 7:00 PM.'),
    from: 'Sarath Shailan',
    at: '5:12 PM',
    acknowledgedBy: ['Arjun Kumar', 'Fathima P', 'Sarath Shailan', 'Samyuktha S', 'Vishnu K'],
  });

  const runTest = useCallback(() => {
    void bridge.runConnectionTest().then(next => {
      setReport(next);
      toast(text('toastTestDone', 'Connection test finished.'), 'neutral');
    }).catch(() => toast(text('toastTestFailed', 'The connection test could not run.'), 'danger'));
  }, [bridge, toast]);

  useEffect(() => {
    let mounted = true;
    void bridge.runConnectionTest().then(next => {
      if (mounted) setReport(next);
    }).catch(() => {
      /* Diagnostics are unavailable outside the desktop app. */
    });
    return () => { mounted = false; };
  }, [bridge]);

  const openGuide = useCallback(() => bridge.openFirewallGuide(), [bridge]);

  return (
    <section className="ss-states">
      <ScreenHeader
        actions={<Button icon="refreshCw" onClick={runTest} size="sm" variant="secondary">{text('runTest', 'Run test')}</Button>}
        description={text('statesDescription', 'The connection, empty, announcement and toast states every screen shares.')}
        eyebrow={text('statesEyebrow', 'SYSTEM STATES')}
        title={text('statesTitle', 'Network and system states')}
      />

      <h3 className="ss-states__title">{text('sectionConnection', 'Connection & network states')}</h3>
      <ConnectionCards all onOpenGuide={openGuide} onRunTest={runTest} peerName="Sarath Shailan" report={report} />

      <div className="ss-states__live">
        <ListGroup section={text('sectionLive', 'Live report')}>
          <ListRow
            icon="network"
            title={text('rowListening', 'Listening')}
            hint={report?.listening ? text('rowListeningOn', 'Port {{port}}', { port: report.port }) : text('rowListeningOff', 'Not listening')}
            trailing={<Badge tone={report?.listening ? 'success' : 'danger'}>{report?.listening ? text('yes', 'Yes') : text('no', 'No')}</Badge>}
          />
          <ListRow
            icon="usersRound"
            title={text('rowPeers', 'Peers discovered')}
            hint={text('rowPeersHint', '{{online}} of {{total}} online', { online: report?.onlinePeers ?? 0, total: report?.peers ?? 0 })}
          />
          <ListRow
            icon="globe"
            title={text('rowAdapters', 'Network adapters')}
            hint={report && report.adapters.length > 0 ? report.adapters.join(text('nameSeparator', ', ')) : text('rowNoAdapters', 'None reported')}
          />
          <ListRow
            icon="zap"
            title={text('rowDiscovery', 'Discovery port')}
            hint={String(report?.discoveryPort ?? 0)}
          />
          <ListRow
            icon="alertCircle"
            title={text('rowWarnings', 'Warnings')}
            hint={report && report.warnings.length > 0 ? report.warnings.join(' ') : text('rowNoWarnings', 'None')}
          />
        </ListGroup>
      </div>

      <h3 className="ss-states__title">{text('sectionEmpty', 'Empty states')}</h3>
      <EmptyStates all query="design" />

      <h3 className="ss-states__title">{text('sectionAnnouncement', 'LAN announcement')}</h3>
      <AnnouncementBanner
        acknowledge={() => {
          setAnnouncement(current => ({ ...current, acknowledgedBy: [...current.acknowledgedBy, 'You'] }));
          toast(text('toastAcknowledged', 'Announcement acknowledged.'), 'success');
        }}
        announcement={announcement}
        isSender
      />

      <h3 className="ss-states__title">{text('sectionToasts', 'Toast stack')}</h3>
      <div className="ss-toast-row">
        <Button
          icon={showToasts ? 'bellOff' : 'bellRing'}
          onClick={() => setShowToasts(current => !current)}
          size="sm"
          variant="secondary"
        >
          {showToasts
            ? text('hideToasts', 'Hide the board toasts')
            : text('showToasts', 'Show the board toasts')}
        </Button>
        <span className="ss-toast-note">
          {text('toastsNote', 'The shell mounts one ToastStack per window; it pins to the corner and each toast dismisses itself unless it needs a decision.')}
        </span>
      </div>
      {showToasts ? <ToastStack demo /> : null}
    </section>
  );
}

export default SystemStates;
