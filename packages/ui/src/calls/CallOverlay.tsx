import { useEffect, useRef, useState, type RefObject } from 'react';
import { Avatar, Icon, cx } from '../components';
import { ns, registerNamespace } from '../i18n';
import shellCatalog from '../i18n/en/shell';
import type { AppSettings } from '../../../bridge/src';
import { ringPattern } from '../ringtone';
import { applySinkId } from '../media';
import type { CallController, LocalCall } from './useCall';
import './calls.css';

registerNamespace('shell', shellCatalog);
const text = ns('shell');

function clock(seconds: number): string {
  const minutes = Math.floor(seconds / 60);
  const rest = seconds % 60;
  return `${minutes}:${rest.toString().padStart(2, '0')}`;
}

function useElapsed(call: LocalCall): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (call.phase !== 'active' && call.phase !== 'calling') return;
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [call.phase]);
  return Math.max(0, Math.floor((now - call.startedAt) / 1000));
}

/** The chosen ringtone, repeated until the call is answered, declined or dismissed. WebAudio, so no asset ships. */
function useRingtone(active: boolean, muted: boolean, ringtone: string) {
  useEffect(() => {
    const pattern = ringPattern(ringtone);
    if (!active || muted || pattern.notes.length === 0) return;
    const AudioCtor = window.AudioContext ?? (window as unknown as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext;
    if (!AudioCtor) return;
    const context = new AudioCtor();
    let stopped = false;
    const tone = (frequency: number, at: number) => {
      if (stopped) return;
      const oscillator = context.createOscillator();
      const gain = context.createGain();
      oscillator.frequency.value = frequency;
      oscillator.type = 'sine';
      gain.gain.setValueAtTime(0, context.currentTime + at);
      gain.gain.linearRampToValueAtTime(0.12, context.currentTime + at + 0.02);
      gain.gain.linearRampToValueAtTime(0, context.currentTime + at + 0.35);
      oscillator.connect(gain).connect(context.destination);
      oscillator.start(context.currentTime + at);
      oscillator.stop(context.currentTime + at + 0.4);
    };
    const ring = () => {
      for (const note of pattern.notes) tone(note.hz, note.at);
    };
    ring();
    const timer = window.setInterval(ring, pattern.repeatMs);
    return () => {
      stopped = true;
      window.clearInterval(timer);
      void context.close();
    };
  }, [active, muted, ringtone]);
}

/** Binds a MediaStream to a <video>/<audio> element, on the output Settings picked where the webview takes one. */
function StreamView({ className, muted, sinkId, stream, video }: { className?: string; muted: boolean; sinkId?: string; stream: MediaStream | null; video: boolean }) {
  const ref = useRef<HTMLVideoElement | HTMLAudioElement | null>(null);
  useEffect(() => {
    const element = ref.current;
    if (!element) return;
    element.srcObject = stream;
    return () => {
      element.srcObject = null;
    };
  }, [stream]);
  useEffect(() => {
    if (ref.current) applySinkId(ref.current, sinkId);
  }, [sinkId]);
  if (!video) {
    // Remote audio plays through a hidden element so voice calls need no video surface.
    return <audio autoPlay ref={ref as RefObject<HTMLAudioElement>} />;
  }
  return <video autoPlay className={className} muted={muted} playsInline ref={ref as RefObject<HTMLVideoElement>} />;
}

function Controls({ call, controller }: { call: LocalCall; controller: CallController }) {
  const showCamera = call.media === 'video';
  return (
    <div className="ac-call__controls">
      <button
        aria-label={call.micOn ? text('callMute', 'Mute') : text('callUnmute', 'Unmute')}
        className={cx('ac-call__control', !call.micOn && 'is-off')}
        onClick={controller.toggleMute}
        type="button"
      >
        <Icon name={call.micOn ? 'mic' : 'micOff'} size={18} />
      </button>
      {showCamera ? (
        <button
          aria-label={call.cameraOn ? text('callCameraOff', 'Turn camera off') : text('callCameraOn', 'Turn camera on')}
          className={cx('ac-call__control', !call.cameraOn && 'is-off')}
          onClick={controller.toggleCamera}
          type="button"
        >
          <Icon name={call.cameraOn ? 'video' : 'videoOff'} size={18} />
        </button>
      ) : null}
      <button
        aria-label={call.sharing ? text('callStopShare', 'Stop sharing') : text('callShare', 'Share screen')}
        className={cx('ac-call__control', call.sharing && 'is-on')}
        onClick={() => void controller.toggleShare()}
        type="button"
      >
        <Icon name={call.sharing ? 'monitorOff' : 'monitor'} size={18} />
      </button>
      <button
        aria-label={call.pip ? text('callExpand', 'Expand call') : text('callPip', 'Picture-in-picture')}
        className="ac-call__control"
        onClick={controller.togglePip}
        type="button"
      >
        <Icon name={call.pip ? 'maximize' : 'minimize'} size={18} />
      </button>
      <button
        aria-label={text('callHangUp', 'Hang up')}
        className="ac-call__control ac-call__control--hangup"
        onClick={controller.hangup}
        type="button"
      >
        <Icon name="phoneOff" size={18} />
      </button>
    </div>
  );
}

function DevicePanel({ call, controller }: { call: LocalCall; controller: CallController }) {
  const [microphones, setMicrophones] = useState<readonly MediaDeviceInfo[]>([]);
  const [cameras, setCameras] = useState<readonly MediaDeviceInfo[]>([]);
  useEffect(() => {
    let live = true;
    void navigator.mediaDevices
      .enumerateDevices()
      .then(devices => {
        if (!live) return;
        setMicrophones(devices.filter(device => device.kind === 'audioinput'));
        setCameras(devices.filter(device => device.kind === 'videoinput'));
      })
      .catch(() => undefined);
    return () => {
      live = false;
    };
  }, []);
  const chosen = (tracks: MediaStreamTrack[] | undefined) =>
    tracks?.[0]?.getSettings().deviceId ?? '';
  return (
    <div className="ac-call__devices">
      <label>
        <span>{text('callMicrophone', 'Microphone')}</span>
        <select
          onChange={event => void controller.switchDevice('microphone', event.target.value)}
          value={chosen(call.localStream?.getAudioTracks())}
        >
          {microphones.map((device, index) => (
            <option key={device.deviceId} value={device.deviceId}>
              {device.label || `${text('callMicrophone', 'Microphone')} ${index + 1}`}
            </option>
          ))}
        </select>
      </label>
      {call.media === 'video' ? (
        <label>
          <span>{text('callCamera', 'Camera')}</span>
          <select
            onChange={event => void controller.switchDevice('camera', event.target.value)}
            value={chosen(call.localStream?.getVideoTracks())}
          >
            {cameras.map((device, index) => (
              <option key={device.deviceId} value={device.deviceId}>
                {device.label || `${text('callCamera', 'Camera')} ${index + 1}`}
              </option>
            ))}
          </select>
        </label>
      ) : null}
    </div>
  );
}

export function CallOverlay({
  controller,
  settings,
}: {
  controller: CallController;
  settings: AppSettings;
}) {
  const { call } = controller;
  const [devicesOpen, setDevicesOpen] = useState(false);
  const elapsed = useElapsed(call);
  useRingtone(call.phase === 'ringing' && call.incoming, settings.notificationsMuted, settings.ringtone);

  const dismiss = controller.dismiss;
  useEffect(() => {
    if (call.phase !== 'ended' && call.phase !== 'failed') return;
    const timer = window.setTimeout(dismiss, 4500);
    return () => window.clearTimeout(timer);
  }, [call.callId, call.phase, dismiss]);

  useEffect(() => {
    if (call.phase === 'idle') setDevicesOpen(false);
  }, [call.phase]);

  if (call.phase === 'idle') return null;

  if (call.phase === 'ringing' && call.incoming) {
    return (
      <div className="ac-call-incoming" role="alertdialog" aria-label={text('callIncoming', 'Incoming call')}>
        <button aria-label={text('callDecline', 'Decline')} className="ac-call-incoming__close" onClick={controller.decline} type="button">
          <Icon name="x" size={14} />
        </button>
        <Avatar name={call.peerName} size={56} />
        <div className="ac-call-incoming__body">
          <strong>{call.peerName}</strong>
          <span>
            <Icon name={call.media === 'video' ? 'video' : 'phone'} size={13} />
            {call.media === 'video'
              ? text('callIncomingVideo', 'Incoming video call')
              : text('callIncomingVoice', 'Incoming voice call')}
          </span>
        </div>
        <div className="ac-call-incoming__actions">
          <button aria-label={text('callDecline', 'Decline')} className="ac-call-round ac-call-round--decline" onClick={controller.decline} type="button">
            <Icon name="phoneOff" size={18} />
          </button>
          <button aria-label={text('callAccept', 'Accept')} className="ac-call-round ac-call-round--accept" onClick={() => void controller.accept()} type="button">
            <Icon name="phone" size={18} />
          </button>
        </div>
      </div>
    );
  }

  if (call.phase === 'ended' || call.phase === 'failed') {
    return (
      <div className={cx('ac-call', 'ac-call--ended')} role="status">
        <Avatar name={call.peerName} size={40} />
        <div className="ac-call__ended-body">
          <strong>{call.peerName}</strong>
          <span>{call.error ?? text('callEnded', 'Ended')}</span>
        </div>
        <button aria-label={text('callDismiss', 'Dismiss')} className="ac-call__control" onClick={controller.dismiss} type="button">
          <Icon name="x" size={16} />
        </button>
      </div>
    );
  }

  const video = call.media === 'video' || call.sharing;
  const label =
    call.phase === 'calling'
      ? text('callCalling', 'Calling…')
      : call.phase === 'ringing'
        ? text('callRinging', 'Ringing')
        : clock(elapsed);

  if (call.pip) {
    return (
      <div className="ac-call-pip">
        {video && call.remoteStream ? (
          <StreamView className="ac-call-pip__video" muted={false} sinkId={settings.speaker} stream={call.remoteStream} video />
        ) : (
          <Avatar name={call.peerName} size={56} />
        )}
        <div className="ac-call-pip__bar">
          <span>{call.peerName}</span>
          <button aria-label={text('callExpand', 'Expand call')} className="ac-call__control" onClick={controller.togglePip} type="button">
            <Icon name="maximize" size={14} />
          </button>
          <button aria-label={text('callHangUp', 'Hang up')} className="ac-call__control ac-call__control--hangup" onClick={controller.hangup} type="button">
            <Icon name="phoneOff" size={14} />
          </button>
        </div>
        {!video ? <StreamView muted={false} sinkId={settings.speaker} stream={call.remoteStream} video={false} /> : null}
      </div>
    );
  }

  return (
    <section className={cx('ac-call', video && 'ac-call--video')} aria-label={text('callWindow', 'Call')}>
      {video ? (
        <StreamView className="ac-call__remote" muted={false} sinkId={settings.speaker} stream={call.remoteStream} video />
      ) : (
        <div className="ac-call__voice">
          <Avatar name={call.peerName} size="lg" />
          <strong>{call.peerName}</strong>
          <span className="ac-call__timer">{label}</span>
          <div className="ac-call__wave" aria-hidden>
            <i />
            <i />
            <i />
            <i />
            <i />
            <i />
            <i />
          </div>
          <StreamView muted={false} sinkId={settings.speaker} stream={call.remoteStream} video={false} />
        </div>
      )}
      {video ? (
        <>
          <div className="ac-call__top">
            <span className="ac-call__peer">
              <span className="ac-call__dot" />
              {call.peerName}
            </span>
            <span className="ac-call__timer">{label}</span>
          </div>
          {call.localStream && call.cameraOn && !call.sharing ? (
            <div className="ac-call__self">
              <StreamView muted stream={call.localStream} video />
            </div>
          ) : null}
        </>
      ) : null}
      {devicesOpen ? <DevicePanel call={call} controller={controller} /> : null}
      <div className="ac-call__bar">
        <button
          aria-label={text('callDevices', 'Devices')}
          className={cx('ac-call__control', devicesOpen && 'is-on')}
          onClick={() => setDevicesOpen(open => !open)}
          type="button"
        >
          <Icon name="settings" size={18} />
        </button>
        <Controls call={call} controller={controller} />
      </div>
    </section>
  );
}

/** Exported for the gallery and tests: the label a call card would show. */
export function callLabel(call: LocalCall): string {
  if (call.phase === 'calling') return text('callCalling', 'Calling…');
  if (call.phase === 'ringing') return text('callRinging', 'Ringing');
  if (call.phase === 'active') return text('callOngoing', 'Ongoing');
  if (call.phase === 'failed') return call.error ?? text('callFailed', 'Failed');
  if (call.phase === 'ended') return text('callEnded', 'Ended');
  return '';
}
