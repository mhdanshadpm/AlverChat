import { describe, expect, it } from 'vitest';
import { dockPriority, shouldShowBubble, visibleBubbles } from './state';
import {
  ACCEPTED_AVATAR_TYPES,
  AVATAR_MAX_BYTES,
  AVATAR_STORED_MAX_BYTES,
  avatarFromDataUrl,
  chooseAvatarQuality,
  clampZoom,
  cropRect,
  dataUrlBytes,
  decodeIconSpec,
  drawAvatar,
  encodeAvatar,
  encodeIconSpec,
  isAcceptedAvatarType,
  normalizeDegrees,
  prepareAvatarDataUrl,
  rotationCoverFactor,
  validateAvatarFile,
  zoomFromPercent,
} from './screens/photoPipeline';

describe('floating dock behavior',()=>{
  it('keeps only the three newest message bubbles',()=>{expect(visibleBubbles([1,2,3,4].map(id=>({id:String(id),sender:'peer',text:'hi',sentAt:id}))).map(b=>b.id)).toEqual(['2','3','4'])});
  it('prioritizes a call over unread and idle states',()=>{expect(dockPriority(true,3)).toBe('call');expect(dockPriority(false,2)).toBe('unread');expect(dockPriority(false,0)).toBe('idle')});
  it('suppresses bubbles for open, muted, and DND conversations',()=>{expect(shouldShowBubble(false,false,false)).toBe(true);expect(shouldShowBubble(true,false,false)).toBe(false);expect(shouldShowBubble(false,true,false)).toBe(false);expect(shouldShowBubble(false,false,true)).toBe(false)});
});

/* ===== photoPipeline (pure parts only — no DOM, no canvas) ===== */

const jpeg = (length: number) => `data:image/jpeg;base64,${'A'.repeat(length)}`;
const gated = (type: string, size: number) => ({ size, type }) as unknown as Parameters<typeof prepareAvatarDataUrl>[0];

describe('avatar upload gate', () => {
  it('accepts only the three raster formats the webview can decode', () => {
    expect(ACCEPTED_AVATAR_TYPES).toEqual(['image/png', 'image/jpeg', 'image/webp']);
    expect(ACCEPTED_AVATAR_TYPES.every(isAcceptedAvatarType)).toBe(true);
    expect(isAcceptedAvatarType('IMAGE/JPEG')).toBe(true);
    expect(isAcceptedAvatarType('image/png;charset=binary')).toBe(true);
    for (const bad of ['image/gif', 'image/svg+xml', 'video/mp4', 'application/octet-stream', '']) {
      expect(isAcceptedAvatarType(bad)).toBe(false);
    }
  });

  it('reports why a file was refused', () => {
    expect(validateAvatarFile({ type: 'image/png', size: 2048 })).toEqual({ ok: true });
    expect(validateAvatarFile({ type: 'image/webp', size: AVATAR_MAX_BYTES })).toEqual({ ok: true });
    expect(validateAvatarFile({ type: 'image/gif', size: 2048 })).toEqual({ ok: false, reason: 'type' });
    expect(validateAvatarFile({ type: 'image/jpeg', size: AVATAR_MAX_BYTES + 1 })).toEqual({ ok: false, reason: 'size' });
    for (const size of [0, -1, Number.NaN]) {
      expect(validateAvatarFile({ type: 'image/png', size })).toEqual({ ok: false, reason: 'empty' });
    }
  });

  it('verifies type and size before decoding anything', async () => {
    // A refused file returns from the gate before FileReader/Image are touched,
    // which is the only reason these resolve in a DOM-free node environment.
    await expect(prepareAvatarDataUrl(gated('image/gif', 4096))).resolves.toEqual({ ok: false, reason: 'type' });
    await expect(prepareAvatarDataUrl(gated('image/png', AVATAR_MAX_BYTES * 2))).resolves.toEqual({ ok: false, reason: 'size' });
    await expect(prepareAvatarDataUrl(gated('image/png', 0))).resolves.toEqual({ ok: false, reason: 'empty' });
  });
});

describe('stored avatar values', () => {
  it('measures a base64 payload without decoding it', () => {
    expect(dataUrlBytes('')).toBe(0);
    expect(dataUrlBytes(jpeg(4))).toBe(3);
    expect(dataUrlBytes(jpeg(8))).toBe(6);
    expect(dataUrlBytes(jpeg(100))).toBe(75);
  });

  it('refuses anything that is not an accepted image data URL', () => {
    const stored = jpeg(88);
    expect(avatarFromDataUrl(null)).toBeNull();
    expect(avatarFromDataUrl('')).toBeNull();
    expect(avatarFromDataUrl('https://evil.example/a.jpg')).toBeNull();
    expect(avatarFromDataUrl('data:image/svg+xml;base64,AAA=')).toBeNull();
    expect(avatarFromDataUrl('data:image/png,not-base-64')).toBeNull();
    expect(avatarFromDataUrl(stored)).toBe(stored);
  });

  it('drops a stored avatar that is bigger than the budget', () => {
    // 700_000 base64 chars decode to ~525_000 bytes, just past the 512 KiB cap.
    expect(avatarFromDataUrl(jpeg(700_000))).toBeNull();
    expect(avatarFromDataUrl(jpeg(600_000))).not.toBeNull();
    expect(AVATAR_STORED_MAX_BYTES).toBe(512 * 1024);
  });
});

describe('group icon specs', () => {
  it('round-trips an emoji + palette colour through the single icon string', () => {
    const encoded = encodeIconSpec({ kind: 'emoji', emoji: '🎨', color: 'cyan' });
    expect(encoded).toBe('ac-emoji:cyan:🎨');
    expect(decodeIconSpec(encoded)).toEqual({ kind: 'emoji', emoji: '🎨', color: 'cyan' });
  });

  it('keeps the plain emoji the mock seeds readable and colours it', () => {
    expect(decodeIconSpec('💰')).toEqual({ kind: 'emoji', emoji: '💰', color: expect.any(String) });
    expect(decodeIconSpec('ac-emoji:not-a-colour:🚀')).toEqual({ kind: 'emoji', emoji: '🚀', color: expect.any(String) });
  });

  it('reads a stored photo back as a photo', () => {
    const src = jpeg(64);
    expect(decodeIconSpec(src)).toEqual({ kind: 'photo', src });
    expect(encodeIconSpec({ kind: 'photo', src })).toBe(src);
  });

  it('rejects malformed or unsafe icons', () => {
    expect(encodeIconSpec(null)).toBeUndefined();
    expect(decodeIconSpec(null)).toBeNull();
    expect(decodeIconSpec('')).toBeNull();
    expect(decodeIconSpec('ac-emoji:violet:')).toBeNull();
    expect(decodeIconSpec('data:text/html;base64,PHNjcmlwdD4=')).toBeNull();
  });
});

describe('crop, rotate and zoom maths', () => {
  it('takes the centre square and shrinks it by the zoom factor', () => {
    expect(cropRect({ sourceWidth: 200, sourceHeight: 200 })).toEqual({ sx: 0, sy: 0, sw: 200, sh: 200 });
    expect(cropRect({ sourceWidth: 400, sourceHeight: 200 })).toEqual({ sx: 100, sy: 0, sw: 200, sh: 200 });
    expect(cropRect({ sourceWidth: 200, sourceHeight: 200, zoom: 2 })).toEqual({ sx: 50, sy: 50, sw: 100, sh: 100 });
    expect(cropRect({ sourceWidth: 0, sourceHeight: 0 })).toEqual({ sx: 0, sy: 0, sw: 1, sh: 1 });
  });

  it('clamps zoom to the range the slider offers', () => {
    expect(zoomFromPercent(100)).toBe(1);
    expect(zoomFromPercent(180)).toBe(1.8);
    expect(zoomFromPercent(50)).toBe(1);
    expect(zoomFromPercent(900)).toBe(2.5);
    expect(clampZoom(Number.NaN)).toBe(1);
  });

  it('normalises degrees and keeps a rotated square covering the disc', () => {
    expect(normalizeDegrees(450)).toBe(90);
    expect(normalizeDegrees(-90)).toBe(270);
    expect(normalizeDegrees(Number.NaN)).toBe(0);
    expect(rotationCoverFactor(0)).toBeCloseTo(1, 6);
    expect(rotationCoverFactor(90)).toBeCloseTo(1, 6);
    expect(rotationCoverFactor(45)).toBeCloseTo(Math.SQRT2, 6);
    expect(rotationCoverFactor(20)).toBeGreaterThan(1);
  });
});

describe('quality fallback chain', () => {
  it('picks the highest quality that still fits the budget', () => {
    const samples = [
      { quality: 0.92, bytes: 690_000 },
      { quality: 0.82, bytes: 615_000 },
      { quality: 0.7, bytes: 300_000 },
    ];
    expect(chooseAvatarQuality(samples, 650_000)).toBe(0.82);
    expect(chooseAvatarQuality(samples, 250_000)).toBe(0.7);
    expect(chooseAvatarQuality([{ quality: 0.92, bytes: 50_000_000 }])).toBe(0.92);
    expect(chooseAvatarQuality([])).toBeNull();
  });

  it('stops encoding as soon as a sample fits and keeps the best one', () => {
    const tried: number[] = [];
    const result = encodeAvatar(
      (quality) => {
        tried.push(quality);
        return jpeg(Math.round(quality * 1000));
      },
      { maxBytes: 650 },
    );
    expect(tried).toEqual([0.92, 0.82]);
    expect(result?.quality).toBe(0.82);
    expect(result?.bytes).toBe(615);
    expect(result?.withinBudget).toBe(true);
  });

  it('ships the softest frame instead of rejecting an enormous picture', () => {
    const result = encodeAvatar(() => jpeg(4_000_000), { qualities: [0.92, 0.58] });
    expect(result?.quality).toBe(0.58);
    expect(result?.withinBudget).toBe(false);
  });

  it('returns null when nothing could be encoded', () => {
    expect(encodeAvatar(() => null)).toBeNull();
  });
});

describe('canvas encoding degrades without a DOM', () => {
  it('drawAvatar returns null instead of throwing in node', () => {
    expect(drawAvatar({ width: 100, height: 100 }, 256)).toBeNull();
    expect(drawAvatar(null)).toBeNull();
    expect(drawAvatar({ width: 100, height: 100 }, 0)).toBeNull();
  });
});
