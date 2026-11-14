import { useCallback, useEffect, useRef, useState } from 'react';
import type { AppSettings, CallSignal, IncomingCallSignal, PlatformBridge } from '../../../bridge/src';

export type CallPhase = 'idle' | 'calling' | 'ringing' | 'active' | 'ended' | 'failed';
export type CallMedia = 'audio' | 'video';

export type LocalCall = {
  callId: string;
  chatId: string;
  peerName: string;
  media: CallMedia;
  phase: CallPhase;
  /** True while the peer is waiting for an answer. */
  incoming: boolean;
  startedAt: number;
  endedAt: number | null;
  micOn: boolean;
  cameraOn: boolean;
  sharing: boolean;
  /** The call card shrunk to its always-on-top corner form. */
  pip: boolean;
  error: string | null;
  localStream: MediaStream | null;
  remoteStream: MediaStream | null;
};

const IDLE: LocalCall = {
  callId: '',
  chatId: '',
  peerName: '',
  media: 'audio',
  phase: 'idle',
  incoming: false,
  startedAt: 0,
  endedAt: null,
  micOn: true,
  cameraOn: true,
  sharing: false,
  pip: false,
  error: null,
  localStream: null,
  remoteStream: null,
};

type Signal = Omit<CallSignal, 'callId'> & { callId: string };

/** Microphone and camera as the settings page selected them. */
function mediaFor(media: CallMedia, settings: AppSettings): MediaStreamConstraints {
  const audio: MediaTrackConstraints = {
    echoCancellation: settings.echoCancellation,
    noiseSuppression: settings.noiseSuppression,
  };
  if (settings.microphone && settings.microphone !== 'default') audio.deviceId = { exact: settings.microphone };
  if (media === 'audio') return { audio, video: false };
  const video: MediaTrackConstraints = { width: { ideal: 1280 }, height: { ideal: 720 } };
  if (settings.camera && settings.camera !== 'default') video.deviceId = { exact: settings.camera };
  return { audio, video };
}

/**
 * One live call at a time, driven by WebRTC and relayed through the encrypted LAN session.
 *
 * The caller is the only side that renegotiates (screen share, camera changes); the
 * receiver answers every offer. That keeps glare out of a two-peer call without needing
 * the full perfect-negotiation rollback machinery.
 */
export function useCall(
  bridge: PlatformBridge,
  settings: AppSettings,
  toast: (message: string, tone?: 'neutral' | 'success' | 'danger') => void,
) {
  const [call, setCall] = useState<LocalCall>(IDLE);
  const callRef = useRef<LocalCall>(IDLE);
  const pcRef = useRef<RTCPeerConnection | null>(null);
  const pendingIce = useRef<RTCIceCandidateInit[]>([]);
  const offerSdp = useRef<string>('');
  const screenStream = useRef<MediaStream | null>(null);
  const settingsRef = useRef(settings);
  settingsRef.current = settings;

  const update = useCallback((patch: Partial<LocalCall>) => {
    callRef.current = { ...callRef.current, ...patch };
    setCall(callRef.current);
  }, []);

  const send = useCallback(
    (chatId: string, signal: Signal) => {
      void bridge.sendCallSignal(chatId, signal).catch(error => {
        toast(String(error), 'danger');
      });
    },
    [bridge, toast],
  );

  const teardown = useCallback(() => {
    screenStream.current?.getTracks().forEach(track => track.stop());
    screenStream.current = null;
    const pc = pcRef.current;
    pcRef.current = null;
    if (pc) {
      pc.onicecandidate = null;
      pc.ontrack = null;
      pc.onnegotiationneeded = null;
      pc.onconnectionstatechange = null;
      pc.getSenders().forEach(sender => sender.track?.stop());
      pc.close();
    }
    callRef.current.localStream?.getTracks().forEach(track => track.stop());
    callRef.current.remoteStream?.getTracks().forEach(track => track.stop());
    pendingIce.current = [];
    offerSdp.current = '';
  }, []);

  const finish = useCallback(
    (phase: CallPhase, error: string | null = null) => {
      teardown();
      update({ phase, endedAt: Date.now(), error, localStream: null, remoteStream: null });
    },
    [teardown, update],
  );

  const attach = useCallback(
    (pc: RTCPeerConnection, chatId: string) => {
      pc.onicecandidate = event => {
        const candidate = event.candidate;
        if (!candidate) return;
        send(chatId, {
          callId: callRef.current.callId,
          kind: 'ice',
          candidate: candidate.candidate,
          sdpMid: candidate.sdpMid,
          sdpIndex: candidate.sdpMLineIndex,
        });
      };
      pc.ontrack = event => {
        const stream = event.streams[0] ?? new MediaStream([event.track]);
        update({ remoteStream: stream });
      };
      pc.onconnectionstatechange = () => {
        if (pc.connectionState === 'failed') finish('failed', 'The connection could not be established.');
      };
    },
    [finish, send, update],
  );

  const local = useCallback(async (media: CallMedia) => {
    try {
      return await navigator.mediaDevices.getUserMedia(mediaFor(media, settingsRef.current));
    } catch (error) {
      const name = error instanceof DOMException ? error.name : '';
      if (name === 'NotAllowedError') throw new Error('Microphone or camera access was denied.');
      if (name === 'NotFoundError') throw new Error('No microphone or camera was found.');
      throw new Error(`Could not start the microphone or camera: ${String(error)}`);
    }
  }, []);

  const start = useCallback(
    async (chatId: string, media: CallMedia, peerName: string) => {
      if (callRef.current.phase === 'calling' || callRef.current.phase === 'active' || callRef.current.phase === 'ringing') {
        toast('A call is already in progress.', 'danger');
        return;
      }
      teardown();
      let stream: MediaStream;
      try {
        stream = await local(media);
      } catch (error) {
        toast(String(error instanceof Error ? error.message : error), 'danger');
        return;
      }
      const callId = crypto.randomUUID();
      const pc = new RTCPeerConnection({ iceServers: [] });
      pcRef.current = pc;
      stream.getTracks().forEach(track => pc.addTrack(track, stream));
      attach(pc, chatId);
      pc.onnegotiationneeded = () => {
        void (async () => {
          if (pc.signalingState !== 'stable') return;
          const offer = await pc.createOffer();
          // The first offer carries the media choice; later ones only renegotiate.
          await pc.setLocalDescription(offer);
          send(chatId, {
            callId,
            kind: 'offer',
            media: callRef.current.sharing ? 'screen' : callRef.current.media,
            sdp: offer.sdp ?? '',
          });
        })();
      };
      callRef.current = {
        ...IDLE,
        callId,
        chatId,
        peerName,
        media,
        phase: 'calling',
        startedAt: Date.now(),
        localStream: stream,
      };
      setCall(callRef.current);
      // Adding tracks above triggers `onnegotiationneeded`, but Safari needs a nudge when
      // the listener is installed after the tracks.
      if (pc.signalingState === 'stable' && !pc.localDescription) {
        const offer = await pc.createOffer();
        await pc.setLocalDescription(offer);
        send(chatId, { callId, kind: 'offer', media, sdp: offer.sdp ?? '' });
      }
    },
    [attach, local, send, teardown, toast],
  );

  const accept = useCallback(async () => {
    const current = callRef.current;
    if (current.phase !== 'ringing' || !offerSdp.current) return;
    let stream: MediaStream;
    try {
      stream = await local(current.media);
    } catch (error) {
      send(current.chatId, { callId: current.callId, kind: 'reject', reason: 'no-media' });
      finish('failed', error instanceof Error ? error.message : String(error));
      return;
    }
    const pc = new RTCPeerConnection({ iceServers: [] });
    pcRef.current = pc;
    stream.getTracks().forEach(track => pc.addTrack(track, stream));
    attach(pc, current.chatId);
    try {
      await pc.setRemoteDescription({ type: 'offer', sdp: offerSdp.current });
      for (const candidate of pendingIce.current.splice(0)) await pc.addIceCandidate(candidate);
      const answer = await pc.createAnswer();
      await pc.setLocalDescription(answer);
      send(current.chatId, { callId: current.callId, kind: 'answer', accepted: true, sdp: answer.sdp ?? '' });
      update({
        phase: 'active',
        incoming: false,
        localStream: stream,
        startedAt: Date.now(),
        error: null,
      });
    } catch (error) {
      finish('failed', `Could not answer the call: ${String(error)}`);
    }
  }, [attach, finish, local, send, update]);

  const decline = useCallback(() => {
    const current = callRef.current;
    if (current.callId) send(current.chatId, { callId: current.callId, kind: 'reject', reason: 'declined' });
    finish('ended');
  }, [finish, send]);

  const hangup = useCallback(() => {
    const current = callRef.current;
    if (current.callId && (current.phase === 'active' || current.phase === 'calling')) {
      send(current.chatId, { callId: current.callId, kind: 'end', reason: 'hangup' });
    }
    finish('ended');
  }, [finish, send]);

  const toggleMute = useCallback(() => {
    const stream = callRef.current.localStream;
    if (!stream) return;
    const next = !callRef.current.micOn;
    stream.getAudioTracks().forEach(track => (track.enabled = next));
    update({ micOn: next });
  }, [update]);

  const toggleCamera = useCallback(() => {
    const stream = callRef.current.localStream;
    if (!stream) return;
    const next = !callRef.current.cameraOn;
    stream.getVideoTracks().forEach(track => (track.enabled = next));
    update({ cameraOn: next });
  }, [update]);

  const restoreCameraTrack = useCallback(async () => {
    const pc = pcRef.current;
    const stream = callRef.current.localStream;
    if (!pc || !stream) return;
    try {
      const fresh = await local('video');
      const track = fresh.getVideoTracks()[0];
      const sender = pc.getSenders().find(item => item.track?.kind === 'video');
      if (track) {
        if (sender) await sender.replaceTrack(track);
        else pc.addTrack(track, stream);
        stream.getVideoTracks().forEach(old => old.stop());
        stream.addTrack(track);
      }
    } catch {
      // The camera may be busy; the share simply stops being replaced.
    }
  }, [local]);

  const toggleShare = useCallback(async () => {
    const current = callRef.current;
    const pc = pcRef.current;
    if (!pc || current.phase !== 'active') return;
    if (current.sharing) {
      screenStream.current?.getTracks().forEach(track => track.stop());
      screenStream.current = null;
      update({ sharing: false });
      await restoreCameraTrack();
      return;
    }
    try {
      const display = await navigator.mediaDevices.getDisplayMedia({ video: true, audio: false });
      const track = display.getVideoTracks()[0];
      if (!track) return;
      screenStream.current = display;
      track.onended = () => {
        screenStream.current = null;
        update({ sharing: false });
        void restoreCameraTrack();
      };
      const sender = pc.getSenders().find(item => item.track?.kind === 'video');
      if (sender) {
        await sender.replaceTrack(track);
      } else {
        pc.addTrack(track, display);
        const offer = await pc.createOffer();
        await pc.setLocalDescription(offer);
        send(current.chatId, {
          callId: current.callId,
          kind: 'offer',
          media: 'screen',
          sdp: offer.sdp ?? '',
        });
      }
      update({ sharing: true });
    } catch (error) {
      const name = error instanceof DOMException ? error.name : '';
      if (name !== 'NotAllowedError') toast(`Screen sharing failed: ${String(error)}`, 'danger');
    }
  }, [restoreCameraTrack, send, toast, update]);

  const togglePip = useCallback(() => update({ pip: !callRef.current.pip }), [update]);

  /** Swaps the live microphone or camera for another device without dropping the call. */
  const switchDevice = useCallback(
    async (kind: 'microphone' | 'camera', deviceId: string) => {
      const pc = pcRef.current;
      const stream = callRef.current.localStream;
      if (!pc || !stream) return;
      const constraints: MediaStreamConstraints =
        kind === 'microphone'
          ? {
              audio: {
                deviceId: { exact: deviceId },
                echoCancellation: settingsRef.current.echoCancellation,
                noiseSuppression: settingsRef.current.noiseSuppression,
              },
              video: false,
            }
          : { audio: false, video: { deviceId: { exact: deviceId } } };
      try {
        const fresh = await navigator.mediaDevices.getUserMedia(constraints);
        const track = kind === 'microphone' ? fresh.getAudioTracks()[0] : fresh.getVideoTracks()[0];
        if (!track) return;
        const sender = pc.getSenders().find(item => item.track?.kind === track.kind);
        if (sender) await sender.replaceTrack(track);
        else pc.addTrack(track, stream);
        const stale = kind === 'microphone' ? stream.getAudioTracks() : stream.getVideoTracks();
        stale.forEach(old => {
          old.stop();
          stream.removeTrack(old);
        });
        stream.addTrack(track);
        update({});
      } catch (error) {
        toast(`Could not switch the ${kind}: ${String(error)}`, 'danger');
      }
    },
    [toast, update],
  );

  const dismiss = useCallback(() => {
    teardown();
    callRef.current = IDLE;
    setCall(IDLE);
  }, [teardown]);

  const handleSignal = useCallback(
    async (incoming: IncomingCallSignal, peerName: string) => {
      const signal = incoming.signal;
      const current = callRef.current;
      if (signal.kind === 'offer') {
        if (current.callId && current.callId !== signal.callId) {
          send(incoming.chatId, { callId: signal.callId, kind: 'busy' });
          return;
        }
        const pc = pcRef.current;
        if (pc && current.callId === signal.callId && current.phase === 'active') {
          // A renegotiation offer (screen share or a camera track being added).
          try {
            await pc.setRemoteDescription({ type: 'offer', sdp: signal.sdp ?? '' });
            const answer = await pc.createAnswer();
            await pc.setLocalDescription(answer);
            send(incoming.chatId, {
              callId: current.callId,
              kind: 'answer',
              accepted: true,
              sdp: answer.sdp ?? '',
            });
          } catch (error) {
            toast(`Could not renegotiate the call: ${String(error)}`, 'danger');
            finish('failed', 'The call connection changed unexpectedly.');
          }
          return;
        }
        // A fresh offer: ring until the user answers.
        teardown();
        offerSdp.current = signal.sdp ?? '';
        callRef.current = {
          ...IDLE,
          callId: signal.callId,
          chatId: incoming.chatId,
          peerName,
          media: signal.media === 'video' ? 'video' : 'audio',
          phase: 'ringing',
          incoming: true,
          startedAt: Date.now(),
        };
        setCall(callRef.current);
        return;
      }
      if (!current.callId || current.callId !== signal.callId) return;
      if (signal.kind === 'answer') {
        if (current.phase === 'active') {
          const pc = pcRef.current;
          if (pc) {
            try {
              await pc.setRemoteDescription({ type: 'answer', sdp: signal.sdp ?? '' });
            } catch {
              // An answer to a renegotiation offer; nothing to change locally.
            }
          }
          return;
        }
        const pc = pcRef.current;
        if (!pc) return;
        try {
          await pc.setRemoteDescription({ type: 'answer', sdp: signal.sdp ?? '' });
          for (const candidate of pendingIce.current.splice(0)) await pc.addIceCandidate(candidate);
          update({ phase: 'active', startedAt: Date.now() });
        } catch (error) {
          finish('failed', `The peer's answer was unusable: ${String(error)}`);
        }
        return;
      }
      if (signal.kind === 'ice') {
        const candidate: RTCIceCandidateInit = {
          candidate: signal.candidate ?? '',
          sdpMid: signal.sdpMid ?? null,
          sdpMLineIndex: signal.sdpIndex ?? null,
        };
        const pc = pcRef.current;
        if (pc && pc.remoteDescription && candidate.candidate) {
          await pc.addIceCandidate(candidate).catch(() => undefined);
        } else if (candidate.candidate) {
          pendingIce.current.push(candidate);
        }
        return;
      }
      if (signal.kind === 'end') {
        finish('ended');
        return;
      }
      if (signal.kind === 'reject') {
        finish('ended', signal.reason === 'declined' ? null : 'The call was declined.');
        return;
      }
      if (signal.kind === 'busy') {
        finish('failed', 'The peer is on another call.');
      }
    },
    [finish, send, teardown, toast, update],
  );

  useEffect(() => {
    if (call.phase === 'idle') return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape' && call.phase === 'active') hangup();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [call.phase, hangup]);

  useEffect(() => () => teardown(), [teardown]);

  return {
    call,
    start,
    accept,
    decline,
    hangup,
    toggleMute,
    toggleCamera,
    toggleShare,
    togglePip,
    switchDevice,
    dismiss,
    handleSignal,
  };
}

export type CallController = ReturnType<typeof useCall>;
