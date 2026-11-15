/* ------------------------------------------------------------------ *
 * Camera, microphone and speaker access.
 *
 * Two rules the browser enforces make device choices awkward, and both are
 * handled here: a stored device id stops existing once the hardware is
 * unplugged, and every engine hides `deviceId` and `label` until the page has
 * live media access — so a picker can only be filled by briefly turning the
 * hardware on. A third rule is not fixable in script: naming an *output* needs
 * `setSinkId`, which no WebKit build implements, so the speaker choice is
 * applied where it exists and quietly left to the system where it does not.
 * ------------------------------------------------------------------ */

/** `openMedia` result, including whether the chosen device had to be dropped. */
export type OpenedMedia = { stream: MediaStream; usedDefault: boolean };

/** What `probeDevices` managed to switch on. */
export type DeviceProbe = 'both' | 'microphone' | 'none';

function trackRequested(track: MediaStreamConstraints['audio']): track is MediaTrackConstraints {
  return typeof track === 'object' && track !== null;
}

function pinsDevice(constraints: MediaStreamConstraints): boolean {
  return [constraints.audio, constraints.video].some((track) => trackRequested(track) && track.deviceId !== undefined);
}

/** Chromium reports a vanished device as `OverconstrainedError`, WebKit as `NotFoundError`. */
function isGoneDevice(error: unknown): boolean {
  return (
    error instanceof DOMException && (error.name === 'OverconstrainedError' || error.name === 'NotFoundError')
  );
}

function withoutDeviceIds(constraints: MediaStreamConstraints): MediaStreamConstraints {
  const relaxed = (track: MediaStreamConstraints['audio']) =>
    trackRequested(track) ? { ...track, deviceId: undefined } : track;
  return { ...constraints, audio: relaxed(constraints.audio), video: relaxed(constraints.video) };
}

/**
 * `getUserMedia` that falls back to whatever the system considers default when a
 * device selected in Settings has been unplugged, so a call is not lost to a
 * stale choice. Any other failure is passed through untouched.
 */
export async function openMedia(
  media: Pick<MediaDevices, 'getUserMedia'>,
  constraints: MediaStreamConstraints,
): Promise<OpenedMedia> {
  if (!pinsDevice(constraints)) {
    return { stream: await media.getUserMedia(constraints), usedDefault: false };
  }
  try {
    return { stream: await media.getUserMedia(constraints), usedDefault: false };
  } catch (error) {
    if (!isGoneDevice(error)) throw error;
    return { stream: await media.getUserMedia(withoutDeviceIds(constraints)), usedDefault: true };
  }
}

/**
 * Asks for camera plus microphone, then microphone alone, and stops everything
 * again. The camera is tried first because it is the stricter grant: if it is
 * refused the mic list still unlocks, which is what Settings needs to show names.
 */
export async function probeDevices(media: Pick<MediaDevices, 'getUserMedia'>): Promise<DeviceProbe> {
  try {
    const both = await media.getUserMedia({ audio: true, video: true });
    const sawVideo = both.getVideoTracks().length > 0;
    both.getTracks().forEach((track) => track.stop());
    return sawVideo ? 'both' : 'microphone';
  } catch {
    /* the camera is unavailable; the microphone may still be */
  }
  try {
    const audio = await media.getUserMedia({ audio: true, video: false });
    audio.getTracks().forEach((track) => track.stop());
    return 'microphone';
  } catch {
    return 'none';
  }
}

/** Devices the engine was willing to name, which is what a picker can offer. */
export function namedDevices(devices: readonly MediaDeviceInfo[]): MediaDeviceInfo[] {
  return devices.filter((device) => device.deviceId && device.label.trim().length > 0);
}

/**
 * Routes a playing element to the output chosen in Settings → Calls. `setSinkId` is the only
 * web API for output selection and WebKit has never shipped it, so on macOS the stored choice
 * waits for a webview that can honour it and the element keeps playing through whatever the
 * system is using; the return value says whether this element was retargeted.
 */
export function applySinkId(element: { setSinkId?: (deviceId: string) => Promise<void> }, sinkId: string | undefined): boolean {
  const id = (sinkId ?? '').trim();
  if (!id || id === 'default' || typeof element.setSinkId !== 'function') return false;
  void element.setSinkId(id).catch(() => {
    /* A refused output leaves the element on the route it already has; playback must not stop. */
  });
  return true;
}
