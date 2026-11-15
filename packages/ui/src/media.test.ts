import { describe, expect, it, vi } from 'vitest';
import { applySinkId, namedDevices, openMedia, probeDevices } from './media';

/* ===== helpers ===== */

function device(deviceId: string, label: string, kind: MediaDeviceKind): MediaDeviceInfo {
  return { deviceId, groupId: 'g', kind, label } as MediaDeviceInfo;
}

function track(): MediaStreamTrack {
  return { stop: vi.fn() } as unknown as MediaStreamTrack;
}

function stream(audio: number, video: number): MediaStream {
  const audioTracks = Array.from({ length: audio }, track);
  const videoTracks = Array.from({ length: video }, track);
  return {
    getAudioTracks: () => audioTracks,
    getVideoTracks: () => videoTracks,
    getTracks: () => [...audioTracks, ...videoTracks],
  } as unknown as MediaStream;
}

/* ===== openMedia ===== */

describe('openMedia', () => {
  it('asks for exactly the device saved in settings', async () => {
    const getUserMedia = vi.fn().mockResolvedValue(stream(1, 0));
    const opened = await openMedia({ getUserMedia }, { audio: { deviceId: { exact: 'mic-1' } }, video: false });
    expect(opened.usedDefault).toBe(false);
    expect(getUserMedia).toHaveBeenCalledTimes(1);
    expect(getUserMedia).toHaveBeenCalledWith({ audio: { deviceId: { exact: 'mic-1' } }, video: false });
  });

  it('retries on the system default when the saved device was unplugged', async () => {
    const getUserMedia = vi
      .fn()
      .mockRejectedValueOnce(new DOMException('gone', 'NotFoundError'))
      .mockResolvedValueOnce(stream(1, 1));
    const opened = await openMedia(
      { getUserMedia },
      { audio: { deviceId: { exact: 'mic-1' }, echoCancellation: true }, video: { deviceId: { exact: 'cam-9' } } },
    );
    expect(opened.usedDefault).toBe(true);
    expect(getUserMedia).toHaveBeenLastCalledWith({ audio: { deviceId: undefined, echoCancellation: true }, video: { deviceId: undefined } });
  });

  it('keeps the other track settings when relaxing a stale choice', async () => {
    const getUserMedia = vi
      .fn()
      .mockRejectedValueOnce(new DOMException('gone', 'OverconstrainedError'))
      .mockResolvedValueOnce(stream(1, 0));
    await openMedia({ getUserMedia }, { audio: { deviceId: { exact: 'mic-1' }, noiseSuppression: false }, video: false });
    const relaxed = getUserMedia.mock.calls[1][0] as MediaStreamConstraints;
    expect((relaxed.audio as MediaTrackConstraints).noiseSuppression).toBe(false);
  });

  it('passes a denial through instead of switching devices', async () => {
    const getUserMedia = vi.fn().mockRejectedValue(new DOMException('blocked', 'NotAllowedError'));
    await expect(openMedia({ getUserMedia }, { audio: { deviceId: { exact: 'mic-1' } } })).rejects.toMatchObject({
      name: 'NotAllowedError',
    });
    expect(getUserMedia).toHaveBeenCalledTimes(1);
  });

  it('does not retry a missing default device because nothing was pinned', async () => {
    const getUserMedia = vi.fn().mockRejectedValue(new DOMException('none found', 'NotFoundError'));
    await expect(openMedia({ getUserMedia }, { audio: true, video: false })).rejects.toThrow('none found');
    expect(getUserMedia).toHaveBeenCalledTimes(1);
  });
});

/* ===== probeDevices ===== */

describe('probeDevices', () => {
  it('reports both devices and stops everything it started', async () => {
    const granted = stream(1, 1);
    const getUserMedia = vi.fn().mockResolvedValue(granted);
    expect(await probeDevices({ getUserMedia })).toBe('both');
    expect(getUserMedia).toHaveBeenCalledWith({ audio: true, video: true });
    for (const track of granted.getTracks()) expect(track.stop).toHaveBeenCalledTimes(1);
  });

  it('settles for the microphone when the camera is refused', async () => {
    const getUserMedia = vi
      .fn()
      .mockRejectedValueOnce(new DOMException('camera off', 'NotAllowedError'))
      .mockResolvedValueOnce(stream(1, 0));
    expect(await probeDevices({ getUserMedia })).toBe('microphone');
    expect(getUserMedia).toHaveBeenNthCalledWith(2, { audio: true, video: false });
  });

  it('reports nothing when both are refused', async () => {
    const getUserMedia = vi.fn().mockRejectedValue(new DOMException('blocked', 'NotAllowedError'));
    expect(await probeDevices({ getUserMedia })).toBe('none');
    expect(getUserMedia).toHaveBeenCalledTimes(2);
  });
});

/* ===== namedDevices ===== */

describe('namedDevices', () => {
  it('keeps only what a picker can actually select', () => {
    const list = [
      device('mic-1', 'Yeti Nano', 'audioinput'),
      device('', '', 'audioinput'),
      device('cam-1', '', 'videoinput'),
      device('spk-1', 'Display Audio', 'audiooutput'),
    ];
    expect(namedDevices(list).map((entry) => entry.deviceId)).toEqual(['mic-1', 'spk-1']);
  });
});

/* ===== applySinkId ===== */

describe('applySinkId', () => {
  it('sends the element to the output Settings chose', () => {
    const setSinkId = vi.fn().mockResolvedValue(undefined);
    expect(applySinkId({ setSinkId }, 'spk-1')).toBe(true);
    expect(setSinkId).toHaveBeenCalledWith('spk-1');
  });

  it('leaves the system output alone for "Default"', () => {
    const setSinkId = vi.fn();
    expect(applySinkId({ setSinkId }, 'default')).toBe(false);
    expect(applySinkId({ setSinkId }, '  ')).toBe(false);
    expect(applySinkId({ setSinkId }, undefined)).toBe(false);
    expect(setSinkId).not.toHaveBeenCalled();
  });

  it('does nothing on an engine without setSinkId, which is every WebKit build', () => {
    expect(applySinkId({}, 'spk-1')).toBe(false);
  });

  it('survives an output unplugged mid-call instead of failing the call', async () => {
    const setSinkId = vi.fn().mockRejectedValue(new DOMException('gone', 'NotFoundError'));
    expect(applySinkId({ setSinkId }, 'spk-1')).toBe(true);
    await Promise.resolve();
    await Promise.resolve();
  });
});
