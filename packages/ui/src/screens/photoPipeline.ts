import { AVATAR_PALETTE, avatarPaletteFor, type AvatarPaletteColor } from '../components/logic';

/* ========================================================================== *
 * Avatar image pipeline (spec 7.7 "Photos: resize to 256 px, strip EXIF,
 * sync by hash, verify type and size before decoding, rate-limit").
 *
 * Everything that decides *what* we accept and *how* we encode it is a pure
 * function over plain values, so `state.test.ts` can cover the gates without a
 * DOM. Only `drawAvatar` / `prepareAvatarDataUrl` touch canvas, Image and
 * FileReader, and both degrade to `null` / a rejection outside a browser.
 * ========================================================================== */

/* ===== Gates ===== */

/** Stored avatars are always this square pixel size (profile + group icons). */
export const AVATAR_PIXELS = 256;

/** Largest upload we will even look at, checked before any decoding. */
export const AVATAR_MAX_BYTES = 10 * 1024 * 1024;

/** Budget for the re-encoded 256 px avatar we hand to the bridge. */
export const AVATAR_STORED_MAX_BYTES = 512 * 1024;

/** Only raster formats the webview can decode without a plugin. */
export const ACCEPTED_AVATAR_TYPES = ['image/png', 'image/jpeg', 'image/webp'] as const;

/** JPEG qualities tried in order until the result fits the stored budget. */
export const AVATAR_ENCODE_QUALITIES = [0.92, 0.82, 0.7, 0.58] as const;

/** The three live preview sizes drawn on the Profile and Group boards. */
export const AVATAR_PREVIEW_SIZES = [56, 40, 24] as const;

/** Zoom range offered by the Edit Photo crop dialog (100% = fit the square). */
export const AVATAR_ZOOM_MIN = 100;
export const AVATAR_ZOOM_MAX = 250;

/** JPEG has no alpha channel, so transparency is flattened onto this colour. */
export const AVATAR_MATTE = 'rgb(255, 255, 255)';

export type AvatarFileRejection = 'empty' | 'type' | 'size';

export type AvatarFileCheck = { ok: true } | { ok: false; reason: AvatarFileRejection };

/** MIME types are matched case-insensitively and without parameters. */
export function isAcceptedAvatarType(type: string): boolean {
  const base = (type ?? '').split(';')[0].trim().toLowerCase();
  return (ACCEPTED_AVATAR_TYPES as readonly string[]).includes(base);
}

/**
 * The pre-decode gate: type and size are verified before any pixel is touched,
 * so a malformed or enormous file never reaches the image decoder.
 */
export function validateAvatarFile(file: { type: string; size: number }): AvatarFileCheck {
  if (!file || !Number.isFinite(file.size) || file.size <= 0) return { ok: false, reason: 'empty' };
  if (!isAcceptedAvatarType(file.type)) return { ok: false, reason: 'type' };
  if (file.size > AVATAR_MAX_BYTES) return { ok: false, reason: 'size' };
  return { ok: true };
}

/* ===== Stored-value helpers ===== */

/** Approximate byte length of a base64 data URL payload (no DOM needed). */
export function dataUrlBytes(dataUrl: string): number {
  if (!dataUrl) return 0;
  const comma = dataUrl.indexOf(',');
  const body = comma >= 0 ? dataUrl.slice(comma + 1) : dataUrl;
  const padding = body.endsWith('==') ? 2 : body.endsWith('=') ? 1 : 0;
  return Math.max(0, Math.floor((body.length * 3) / 4) - padding);
}

const SAFE_AVATAR_DATA_URL = /^data:image\/(?:png|jpe?g|webp);base64,[a-z0-9+/=]+$/i;

/**
 * Turns a stored string into something safe to hand to `<img src>`.
 * Anything that is not an accepted base64 image data URL within the stored
 * budget returns `null`, so a corrupt or hostile avatar never gets decoded.
 */
export function avatarFromDataUrl(dataUrl: string | null | undefined): string | null {
  if (!dataUrl) return null;
  if (!SAFE_AVATAR_DATA_URL.test(dataUrl)) return null;
  if (dataUrlBytes(dataUrl) > AVATAR_STORED_MAX_BYTES) return null;
  return dataUrl;
}

/* ===== Icon specs (group icons: uploaded picture OR emoji + palette colour) ===== */

/** Prefix that marks a group icon as "emoji on a palette colour". */
export const EMOJI_ICON_PREFIX = 'ac-emoji:';

export type IconSpec =
  | { kind: 'photo'; src: string }
  | { kind: 'emoji'; emoji: string; color: AvatarPaletteColor };

export function isPaletteColor(value: string): value is AvatarPaletteColor {
  return (AVATAR_PALETTE as readonly string[]).includes(value);
}

/** Serialises an icon choice into the single `icon` string the bridge stores. */
export function encodeIconSpec(spec: IconSpec | null): string | undefined {
  if (!spec) return undefined;
  if (spec.kind === 'photo') return spec.src;
  return `${EMOJI_ICON_PREFIX}${spec.color}:${spec.emoji}`;
}

/**
 * Reads back a stored icon. Plain emoji (the mock seeds `'🎨'`) stay valid and
 * get a stable palette colour derived from the emoji itself.
 */
export function decodeIconSpec(icon: string | null | undefined): IconSpec | null {
  if (!icon) return null;
  const photo = avatarFromDataUrl(icon);
  if (photo) return { kind: 'photo', src: photo };
  if (icon.startsWith(EMOJI_ICON_PREFIX)) {
    const rest = icon.slice(EMOJI_ICON_PREFIX.length);
    const colon = rest.indexOf(':');
    if (colon < 0) return null;
    const color = rest.slice(0, colon);
    const emoji = rest.slice(colon + 1);
    if (emoji.length === 0) return null;
    return { kind: 'emoji', emoji, color: isPaletteColor(color) ? color : avatarPaletteFor(emoji) };
  }
  if (icon.startsWith('data:')) return null;
  return { kind: 'emoji', emoji: icon, color: avatarPaletteFor(icon) };
}

/* ===== Crop + rotate maths ===== */

export type CropRect = { sx: number; sy: number; sw: number; sh: number };

/** Centre square of the source, shrunk by the zoom factor (1 = no zoom). */
export function cropRect(input: {
  sourceWidth: number;
  sourceHeight: number;
  zoom?: number;
}): CropRect {
  const width = Math.max(1, input.sourceWidth || 1);
  const height = Math.max(1, input.sourceHeight || 1);
  const zoom = clampZoom(input.zoom ?? 1);
  const side = Math.min(width, height) / zoom;
  return { sx: (width - side) / 2, sy: (height - side) / 2, sw: side, sh: side };
}

export function clampZoom(zoom: number): number {
  if (!Number.isFinite(zoom)) return 1;
  return Math.min(AVATAR_ZOOM_MAX / 100, Math.max(1, zoom));
}

export function zoomFromPercent(percent: number): number {
  if (!Number.isFinite(percent)) return 1;
  return clampZoom(percent / 100);
}

export function normalizeDegrees(degrees: number): number {
  if (!Number.isFinite(degrees)) return 0;
  return ((degrees % 360) + 360) % 360;
}

/**
 * Multiplier that keeps a rotated square covering a square canvas:
 * 1 at 0/90/180/270 degrees, sqrt(2) at 45 degrees.
 */
export function rotationCoverFactor(degrees: number): number {
  const radians = (normalizeDegrees(degrees) * Math.PI) / 180;
  return Math.abs(Math.cos(radians)) + Math.abs(Math.sin(radians));
}

/* ===== Quality fallback chain ===== */

export type QualitySample = { quality: number; bytes: number };

/**
 * Highest quality that still fits the budget. Falls back to the lowest offered
 * quality when nothing fits (we would rather ship a softer avatar than reject
 * the upload outright), and `null` for an empty sample list.
 */
export function chooseAvatarQuality(
  samples: readonly QualitySample[],
  maxBytes = AVATAR_STORED_MAX_BYTES,
): number | null {
  if (samples.length === 0) return null;
  const fitting = samples.filter((sample) => Number.isFinite(sample.bytes) && sample.bytes <= maxBytes);
  if (fitting.length > 0) {
    return fitting.reduce((best, sample) => (sample.quality > best.quality ? sample : best)).quality;
  }
  return samples.reduce((lowest, sample) => (sample.quality < lowest.quality ? sample : lowest)).quality;
}

export type AvatarEncodeResult = {
  dataUrl: string;
  quality: number;
  bytes: number;
  withinBudget: boolean;
};

/**
 * Drives the quality chain against a caller-supplied encoder so the loop itself
 * is testable with a fake `encode` callback.
 */
export function encodeAvatar(
  encode: (quality: number) => string | null,
  options: { qualities?: readonly number[]; maxBytes?: number } = {},
): AvatarEncodeResult | null {
  const qualities = options.qualities ?? (AVATAR_ENCODE_QUALITIES as readonly number[]);
  const maxBytes = options.maxBytes ?? AVATAR_STORED_MAX_BYTES;
  const samples: Array<{ quality: number; dataUrl: string; bytes: number }> = [];
  for (const quality of qualities) {
    const dataUrl = encode(quality);
    if (dataUrl) samples.push({ quality, dataUrl, bytes: dataUrlBytes(dataUrl) });
    if (samples.length > 0 && samples[samples.length - 1].bytes <= maxBytes) break;
  }
  if (samples.length === 0) return null;
  const chosen = samples.find((sample) => sample.quality === chooseAvatarQuality(samples, maxBytes));
  const result = chosen ?? samples[0];
  return {
    dataUrl: result.dataUrl,
    quality: result.quality,
    bytes: result.bytes,
    withinBudget: result.bytes <= maxBytes,
  };
}

/* ===== Canvas drawing (the only DOM-bound part) ===== */

export type AvatarSource = { width: number; height: number };

export type DrawAvatarOptions = {
  /** 1 = the source centre square, up to AVATAR_ZOOM_MAX/100. */
  zoom?: number;
  /** Degrees; the crop disc is guaranteed to stay covered. */
  rotate?: number;
  quality?: number;
};

type PixelCanvas = {
  width: number;
  height: number;
  getContext: (id: string) => unknown;
  toDataURL?: (type?: string, quality?: number) => string;
};

function createCanvas(size: number): PixelCanvas | null {
  if (typeof document !== 'undefined' && typeof document.createElement === 'function') {
    const canvas = document.createElement('canvas');
    canvas.width = size;
    canvas.height = size;
    return canvas as unknown as PixelCanvas;
  }
  const ctor = (globalThis as { OffscreenCanvas?: new (w: number, h: number) => PixelCanvas }).OffscreenCanvas;
  if (!ctor) return null;
  try {
    return new ctor(size, size);
  } catch {
    return null;
  }
}

type Canvas2d = {
  fillStyle: string;
  save: () => void;
  restore: () => void;
  translate: (x: number, y: number) => void;
  rotate: (radians: number) => void;
  fillRect: (x: number, y: number, w: number, h: number) => void;
  drawImage: (
    source: unknown,
    sx: number,
    sy: number,
    sw: number,
    sh: number,
    dx: number,
    dy: number,
    dw: number,
    dh: number,
  ) => void;
};

/**
 * Draws `source` into a square canvas and returns a JPEG data URL. Re-encoding
 * through canvas is what strips the EXIF orientation/GPS block, so the stored
 * avatar carries no metadata at all. Returns `null` where no canvas exists.
 */
export function drawAvatar(
  source: AvatarSource | null | undefined,
  size = AVATAR_PIXELS,
  options: DrawAvatarOptions = {},
): string | null {
  if (!source || typeof size !== 'number' || size <= 0) return null;
  const canvas = createCanvas(Math.round(size));
  if (!canvas) return null;
  const context = canvas.getContext('2d') as Canvas2d | null;
  if (!context || typeof canvas.toDataURL !== 'function') return null;

  const side = Math.round(size);
  const rect = cropRect({ sourceWidth: source.width, sourceHeight: source.height, zoom: options.zoom });
  const rotate = normalizeDegrees(options.rotate ?? 0);
  const draw = side * rotationCoverFactor(rotate);

  context.fillStyle = AVATAR_MATTE;
  context.fillRect(0, 0, side, side);
  context.save();
  context.translate(side / 2, side / 2);
  if (rotate !== 0) context.rotate((rotate * Math.PI) / 180);
  context.drawImage(source, rect.sx, rect.sy, rect.sw, rect.sh, -draw / 2, -draw / 2, draw, draw);
  context.restore();

  return canvas.toDataURL('image/jpeg', options.quality ?? AVATAR_ENCODE_QUALITIES[0]);
}

/* ===== File -> stored data URL ===== */

export type PrepareAvatarResult =
  | { ok: true; dataUrl: string; bytes: number; quality: number; withinBudget: boolean }
  | { ok: false; reason: AvatarFileRejection };

function readFile(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    if (typeof FileReader === 'undefined') {
      reject(new Error('FileReader is unavailable in this runtime.'));
      return;
    }
    const reader = new FileReader();
    reader.onload = () => resolve(typeof reader.result === 'string' ? reader.result : '');
    reader.onerror = () => reject(reader.error ?? new Error('The file could not be read.'));
    reader.readAsDataURL(blob);
  });
}

function loadImage(src: string): Promise<{ width: number; height: number }> {
  return new Promise((resolve, reject) => {
    if (typeof Image === 'undefined') {
      reject(new Error('Image decoding is unavailable in this runtime.'));
      return;
    }
    const image = new Image();
    image.onload = () => resolve({ width: image.naturalWidth || image.width, height: image.naturalHeight || image.height });
    image.onerror = () => reject(new Error('The image could not be decoded.'));
    image.src = src;
  });
}

/**
 * Full pipeline for one upload: gate first, decode second, then downscale to a
 * 256 px JPEG (EXIF-free by construction) and pick the best quality that fits.
 */
export async function prepareAvatarDataUrl(
  file: { type: string; size: number } & Blob,
  options: { zoom?: number; rotate?: number; pixels?: number } = {},
): Promise<PrepareAvatarResult> {
  const check = validateAvatarFile(file);
  if (!check.ok) return check;
  try {
    const raw = await readFile(file);
    if (!raw) return { ok: false, reason: 'empty' };
    const decoded = await loadImage(raw);
    const pixels = options.pixels ?? AVATAR_PIXELS;
    const encoded = encodeAvatar((quality) => drawAvatar(decoded, pixels, { ...options, quality }));
    if (!encoded) return { ok: false, reason: 'empty' };
    return {
      ok: true,
      dataUrl: encoded.dataUrl,
      bytes: encoded.bytes,
      quality: encoded.quality,
      withinBudget: encoded.withinBudget,
    };
  } catch {
    return { ok: false, reason: 'type' };
  }
}

/** Opens the native picker without putting a raw <input> in the markup. */
export function pickImageFile(accept = ACCEPTED_AVATAR_TYPES.join(',')): Promise<File | null> {
  return new Promise((resolve) => {
    if (typeof document === 'undefined') {
      resolve(null);
      return;
    }
    const input = document.createElement('input');
    input.type = 'file';
    input.accept = accept;
    input.multiple = false;
    input.style.display = 'none';
    input.addEventListener('change', () => {
      const file = input.files && input.files[0] ? input.files[0] : null;
      input.remove();
      resolve(file);
    });
    input.addEventListener('cancel', () => {
      input.remove();
      resolve(null);
    });
    document.body.append(input);
    input.click();
  });
}

/** The first file of a drop event, or null when the drop carried no files. */
export function droppedImage(event: { dataTransfer?: DataTransfer | null }): File | null {
  const files = event.dataTransfer?.files;
  if (!files || files.length === 0) return null;
  return files[0] ?? null;
}
