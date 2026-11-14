import { useCallback, useEffect, useMemo, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import type { CallEvent, CallStatus } from '../../../bridge/src';
import { Avatar, EmptyState, Icon, IconButton, ListRow, Segmented, cx, formatDuration, initialsFor } from '../components';
import { ns, registerNamespace } from '../i18n';
import shellCatalog from '../i18n/en/shell';
import { ScreenHeader, type ScreenProps } from './shared';
import './CallsScreen.css';

registerNamespace('shell', shellCatalog);
const text = ns('shell');

type CallFilter = 'all' | 'missed' | 'group';

const STATUS_LABEL: Record<CallStatus, [string, string]> = {
  missed: ['callMissed', 'Missed'],
  declined: ['callDeclined', 'Declined'],
  ended: ['callEnded', 'Ended'],
  ringing: ['callRinging', 'Ringing'],
  ongoing: ['callOngoing', 'Ongoing'],
  failed: ['callFailed', 'Failed'],
};

function durationOf(call: CallEvent): string {
  if (!call.endedMs || call.status === 'missed' || call.status === 'declined') return '';
  return formatDuration(Math.max(0, (call.endedMs - call.startedMs) / 1000));
}

function stampOf(ms: number): string {
  const at = new Date(ms);
  const sameDay = at.toDateString() === new Date().toDateString();
  return sameDay
    ? at.toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' })
    : at.toLocaleDateString([], { month: 'short', day: 'numeric' });
}

export function CallsScreen({ bridge, peers, go, toast, identity, onCall }: ScreenProps) {
  const [calls, setCalls] = useState<readonly CallEvent[]>([]);
  const [loading, setLoading] = useState(true);
  const [filter, setFilter] = useState<CallFilter>('all');

  const load = useCallback(async () => {
    setLoading(true);
    try {
      setCalls(await bridge.listCalls());
    } catch {
      setCalls([]);
    } finally {
      setLoading(false);
    }
  }, [bridge]);

  useEffect(() => {
    void load();
  }, [load]);

  /* A call that just ended must reach the history without a manual reload. */
  useEffect(() => {
    const tauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
    if (!tauri) return;
    let alive = true;
    let stop: (() => void) | null = null;
    void listen('calls-changed', () => {
      if (alive) void load();
    }).then(unlisten => {
      if (alive) stop = unlisten;
      else unlisten();
    });
    return () => {
      alive = false;
      stop?.();
    };
  }, [load]);

  const visible = useMemo(
    () =>
      calls
        .filter(call => {
          if (filter === 'missed') return call.status === 'missed';
          if (filter === 'group') return call.kind === 'group';
          return true;
        })
        .sort((left, right) => right.startedMs - left.startedMs),
    [calls, filter],
  );

  const missed = calls.filter(call => call.status === 'missed').length;

  const nameFor = useCallback(
    (call: CallEvent) => {
      const peer = peers.find(entry => entry.id === call.chatId);
      return peer?.name ?? call.callerName ?? call.chatId;
    },
    [peers],
  );

  const startCall = useCallback(
    (media: 'audio' | 'video') => {
      const target = peers.find(peer => peer.online && !peer.group);
      if (!target) {
        toast(text('callNoPeer', 'No online peer to call yet.'), 'danger');
        return;
      }
      if (!onCall) {
        toast(text('callUnavailable', 'Calls need the desktop app.'), 'danger');
        return;
      }
      onCall(target.id, media);
    },
    [onCall, peers, toast],
  );

  return (
    <div className={cx('ac-calls')}>
      <ScreenHeader
        eyebrow={text('callsEyebrow', 'LAN CALLING')}
        title={text('callsTitle', 'Calls')}
        description={text(
          'callsBody',
          'Voice, video and screen sharing run directly between the two peers on your local network. No server carries the media.',
        )}
      />

      <section className="ac-calls__start">
        <span className="ac-calls__art">
          <Icon name="phoneCall" size={30} />
        </span>
        <h3>{text('callStart', 'Start a call')}</h3>
        <p>{text('callStartBody', 'Calls stay on your local network — no server ever carries the audio.')}</p>
        <div className="ac-calls__start-actions">
          <button className="primary" onClick={() => startCall('audio')} type="button">
            <Icon name="phone" size={15} />
            {text('callVoice', 'Voice call')}
          </button>
          <button className="soft-button" onClick={() => startCall('video')} type="button">
            <Icon name="video" size={15} />
            {text('callVideo', 'Video call')}
          </button>
        </div>
      </section>

      <div className="ac-calls__toolbar">
        <h4 className="ac-calls__history">{text('callHistory', 'Call history')}</h4>
        <Segmented
          label={text('callFilter', 'Filter calls')}
          onChange={value => setFilter(value as CallFilter)}
          options={[
            { label: text('filterAll', 'All'), value: 'all' as CallFilter },
            { label: text('callMissed', 'Missed'), value: 'missed' as CallFilter },
            { label: text('callGroups', 'Group'), value: 'group' as CallFilter },
          ]}
          value={filter}
        />
      </div>

      {loading ? (
        <div className="ac-calls__loading" role="status">
          <Icon name="loader" size={18} spin />
          {text('callLoading', 'Loading call history…')}
        </div>
      ) : visible.length === 0 ? (
        <div className="ac-calls__empty">
          <EmptyState
            compact
            icon="phoneOff"
            message={text('callsEmptyBody', 'Missed, declined and ended calls appear here.')}
            title={text('callsEmpty', 'No call history')}
          />
        </div>
      ) : (
        <div className="ac-calls__list">
          {visible.map(call => {
            const [key, fallback] = STATUS_LABEL[call.status];
            const outgoing = Boolean(identity?.peerId) && call.caller === identity?.peerId;
            return (
              <div className={cx('ac-call-row', call.status === 'missed' && 'ac-call-row--missed')} key={call.id}>
                <Avatar
                  initials={initialsFor(nameFor(call))}
                  name={nameFor(call)}
                  group={call.kind === 'group'}
                  size={40}
                />
                <span className="ac-call-row__copy">
                  <strong>{nameFor(call)}</strong>
                  <small>
                    <Icon name={outgoing ? 'arrowRight' : 'arrowLeft'} size={12} />
                    {text(key, fallback)}
                    {durationOf(call) ? ` · ${durationOf(call)}` : ''}
                    {call.kind === 'group' ? ` · ${text('callGroup', 'Group call')}` : ''}
                  </small>
                </span>
                <time className="ac-call-row__time">{stampOf(call.startedMs)}</time>
                <span className="ac-call-row__actions">
                  <IconButton
                    iconSize={15}
                    label={text('callBack', 'Call back')}
                    name="phone"
                    onClick={() => {
                      if (call.kind === 'group' || !onCall) {
                        toast(text('callUnavailable', 'Calls are unavailable until LAN call signaling is connected.'), 'danger');
                        return;
                      }
                      onCall(call.chatId, 'audio');
                    }}
                    variant="soft"
                  />
                  <IconButton
                    iconSize={15}
                    label={text('openChat', 'Open chat')}
                    name="messageCircle"
                    onClick={() => go('chats', call.chatId)}
                    variant="soft"
                  />
                </span>
              </div>
            );
          })}
        </div>
      )}

      {missed > 0 ? (
        <ListRow
          hint={text('callMissedHint', 'Tap to see every missed call in one list.')}
          icon="bellRing"
          onClick={() => setFilter('missed')}
          title={text('callMissedCount', '{{count}} missed calls', { count: missed })}
          trailing={<Icon name="chevronRight" size={16} />}
        />
      ) : null}
    </div>
  );
}

export default CallsScreen;
