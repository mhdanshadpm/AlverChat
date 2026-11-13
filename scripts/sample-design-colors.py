"""Sample AlverChat UI colour tokens from the design board pixels.

Deterministic (no RNG, no network, no downloads): re-running writes a
byte-identical docs/design-colors.json because "generatedAt" is derived from
the newest input file's mtime instead of the wall clock.

    python3 scripts/sample-design-colors.py            # sample + write + report
    python3 scripts/sample-design-colors.py --no-write # dry run
"""
from collections import Counter
from pathlib import Path
import datetime as dt
import json
import math
import re
import sys

import numpy as np
from PIL import Image

ROOT = Path(__file__).resolve().parents[1]
BOARDS = sorted((ROOT / 'design').glob('*.png'))
OUT = ROOT / 'docs' / 'design-colors.json'
CSS = ROOT / 'packages' / 'ui' / 'src' / 'styles.css'
DELTA_E_GATE = 6.0  # only tokens differing by more than this are "wrong"

# Three fixed regions a human can eyeball; (board stem, label, box, expectation).
SANITY_REGIONS = [
    ('LAN_Chat_UI_Themes_and_States', 'light chat card (top-left window)', (300, 100, 460, 190), 'light surface'),
    ('LAN_Chat_UI_Themes_and_States', 'dark chat panel (top-right window)', (1050, 120, 1250, 220), 'dark surface'),
    ('AlverChat_Settings_Design_System', 'primary button fill (bottom row)', (1255, 968, 1330, 995), 'brand accent'),
]


def rgb2lab(rgb):
    """sRGB (0-255) -> CIE L*a*b* (D65), vectorised over the last axis."""
    c = np.asarray(rgb, dtype=np.float64) / 255.0
    c = np.where(c <= 0.04045, c / 12.92, ((c + 0.055) / 1.055) ** 2.4)
    x = c @ np.array([0.4124, 0.3576, 0.1805]) / 0.95047
    y = c @ np.array([0.2126, 0.7152, 0.0722])
    z = c @ np.array([0.0193, 0.1192, 0.9505]) / 1.08883
    f = lambda t: np.where(t > 0.008856, np.cbrt(t), 7.787 * t + 16.0 / 116.0)
    fx, fy, fz = f(x), f(y), f(z)
    return np.stack([116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)], axis=-1)


def chroma_hue(lab):
    lab = np.atleast_2d(lab)
    return np.hypot(lab[:, 1], lab[:, 2]), np.degrees(np.arctan2(lab[:, 2], lab[:, 1])) % 360.0


def de_lab(lab1, lab2):
    return _de2000(*np.asarray(lab1, dtype=float), *np.asarray(lab2, dtype=float))


def delta_e(rgb1, rgb2):
    """CIEDE2000 between two sRGB triples."""
    pair = rgb2lab(np.array([rgb1, rgb2], dtype=float))
    return _de2000(*pair[0], *pair[1])


def _de2000(l1, a1, b1, l2, a2, b2):
    c1, c2 = math.hypot(a1, b1), math.hypot(a2, b2)
    cb = (c1 + c2) / 2.0
    g = 0.5 * (1 - math.sqrt(cb ** 7 / (cb ** 7 + 25.0 ** 7))) if cb else 0.0
    h = lambda a, b: math.degrees(math.atan2(b, a)) % 360.0
    a1p, a2p = (1 + g) * a1, (1 + g) * a2
    c1p, c2p = math.hypot(a1p, b1), math.hypot(a2p, b2)
    h1p, h2p = h(a1p, b1), h(a2p, b2)
    dl = (l2 - l1) / 1.0
    dc = c2p - c1p
    dh = 0.0 if c1p * c2p == 0 else (h2p - h1p if abs(h2p - h1p) <= 180 else
                                    (h2p - h1p + 360 if h2p < h1p else h2p - h1p - 360))
    dhp = 2 * math.sqrt(c1p * c2p) * math.sin(math.radians(dh / 2.0))
    lb = (l1 + l2) / 2.0
    cbp = (c1p + c2p) / 2.0
    if c1p * c2p == 0:
        hb = h1p + h2p
    else:
        d = abs(h1p - h2p)
        hb = (h1p + h2p) / 2.0 if d <= 180 else ((h1p + h2p + 360) / 2.0 if h1p + h2p < 360 else (h1p + h2p - 360) / 2.0)
    t = 1 - 0.17 * math.cos(math.radians(hb - 30)) + 0.24 * math.cos(math.radians(2 * hb)) \
        + 0.32 * math.cos(math.radians(3 * hb + 6)) - 0.20 * math.cos(math.radians(4 * hb - 63))
    sl = 1 + 0.015 * (lb - 50) ** 2 / math.sqrt(20 + (lb - 50) ** 2)
    sc = 1 + 0.045 * cbp
    sh = 1 + 0.015 * cbp * t
    rt = -2 * math.sqrt(cbp ** 7 / (cbp ** 7 + 25.0 ** 7)) * \
        math.sin(math.radians(60 * math.exp(-(((hb - 275) / 25) ** 2))))
    return math.sqrt((dl / sl) ** 2 + (dc / sc) ** 2 + (dhp / sh) ** 2 + rt * (dc / sc) * (dhp / sh))


def box_mean(a, k):
    """Mean of a (2k+1)^2 window per pixel, per channel (separable integral image)."""
    h, w = a.shape[:2]
    p = np.pad(a, ((k, k), (k, k), (0, 0)), mode='edge')
    c = np.zeros((h + 2 * k + 1, w + 2 * k + 1, a.shape[2]))
    c[1:, 1:] = np.cumsum(np.cumsum(p, axis=0), axis=1)
    lo_i, lo_j = np.arange(h)[:, None], np.arange(w)[None, :]
    hi_i, hi_j = lo_i + 2 * k + 1, lo_j + 2 * k + 1
    return (c[hi_i, hi_j] - c[lo_i, hi_j] - c[hi_i, lo_j] + c[lo_i, lo_j]) / (2 * k + 1) ** 2


def load_board(path):
    a = np.asarray(Image.open(path).convert('RGB'), dtype=np.float32)
    border = np.concatenate([a[0].reshape(-1, 3), a[1].reshape(-1, 3), a[-1].reshape(-1, 3),
                             a[-2].reshape(-1, 3), a[:, 0], a[:, 1], a[:, -1], a[:, -2]])
    canvas = np.median(border, axis=0)                       # the board's own lavender
    lab = rgb2lab(a)
    ch = np.stack(chroma_hue(lab.reshape(-1, 3)), axis=-1).reshape(*lab.shape[:2], 2)
    lch = np.concatenate([lab, ch], axis=-1)
    mean9 = box_mean(a, 4)
    std9 = np.sqrt(np.maximum(box_mean(a * a, 4) - mean9 ** 2, 0)).mean(axis=2)
    std3 = np.sqrt(np.maximum(box_mean(a * a, 1) - box_mean(a, 1) ** 2, 0)).mean(axis=2)
    off = np.abs(a - canvas).sum(axis=2) >= 40.0             # not the board background
    return dict(a=a, lab=lch, std9=std9, std3=std3, canvas=canvas, off=off,
                flat=off & (std9 < 5.0), smooth=off & (std3 < 7.0),
                mean_lab=rgb2lab(mean9))


def clusters(img, mask, min_count, bins=(5.0, 6.0, 6.0)):
    """Rank the flat colour blobs of a mask: [(rgb, count, L, C, h), ...] by pixel count."""
    px = img['a'][mask]
    if not len(px):
        return []
    key = np.clip(np.round(rgb2lab(px) / np.array(bins)).astype(np.int32), -64, 63)
    packed = (key[:, 0] + 64) * 16512 + (key[:, 1] + 64) * 128 + (key[:, 2] + 64)
    vals, counts = np.unique(packed, return_counts=True)
    out = []
    for v, n in sorted(zip(vals.tolist(), counts.tolist()), key=lambda z: -z[1])[:80]:
        if n < min_count:
            continue
        sel = px[packed == v]
        rgb = np.median(sel, axis=0)
        l, c, h = rgb2lab(rgb[None])[0]
        out.append((rgb, n, float(l), float(c), float(h)))
    return out


def peak(img, mask_lo, bins=5.0):
    """Most populous colour (Lab mode) inside an already filtered pixel set."""
    px = img['a'][mask_lo]
    if len(px) < 20:
        return None, 0
    lab = rgb2lab(px)
    key = np.round(lab / bins).astype(np.int32)
    vals, counts = np.unique(key[:, 0] * 65536 + key[:, 1] * 256 + key[:, 2], return_counts=True)
    best = vals[np.argmax(counts)]
    sel = px[key[:, 0] * 65536 + key[:, 1] * 256 + key[:, 2] == best]
    return np.median(sel, axis=0), int(counts.max())


def tiers(cands, gap, n=3):
    """Pick n well-separated lightness tiers from count-ranked clusters."""
    picked = []
    for cand in cands:
        if all(abs(cand[2] - p[2]) >= gap for p in picked):
            picked.append(cand)
        if len(picked) == n:
            break
    return picked


def ink(img, bg_l_range, ink_l_range, darkest):
    """Text/line colours: ink cores found at the shoulders of a surface of known lightness."""
    own_l, own_c = img['lab'][..., 0], img['lab'][..., 3]
    sur_l = img['mean_lab'][..., 0]                      # lightness of the 9x9 neighbourhood
    near = (sur_l >= bg_l_range[0]) & (sur_l <= bg_l_range[1])
    want = (own_l >= ink_l_range[0]) & (own_l <= ink_l_range[1]) & (own_c < 30)
    sel = np.where((img['std9'] > 12) & img['off'] & near & want)
    if len(sel[0]) < 25:
        return None, 0
    px = img['a'][sel]
    order = np.argsort(rgb2lab(px)[:, 0], kind='stable')
    cut = max(20, int(len(px) * 0.08))
    core = px[order[:cut]] if darkest else px[order[-cut:]]
    return peak({'a': core}, np.ones(len(core), dtype=bool))


def status(img, lo, hi, min_sat, min_val, max_val=0.99):
    """Saturated UI blobs (status dots, badges) inside an HSV hue window."""
    px = img['a'][img['smooth']] / 255.0
    mx = px.max(axis=1)
    d = np.maximum(mx - px.min(axis=1), 1e-6)
    sat = d / np.maximum(mx, 1e-6)
    r, g, b = px[:, 0], px[:, 1], px[:, 2]
    h = np.where(mx == r, ((g - b) / d) % 6, np.where(mx == g, ((b - r) / d) + 2, ((r - g) / d) + 4)) * 60.0
    band = (h >= lo) & (h < hi) if hi <= 360 else ((h >= lo) | (h < hi - 360))
    sel = px[band & (sat > min_sat) & (mx > min_val) & (mx < max_val)] * 255.0
    if len(sel) < 12:
        return None, 0
    return peak({'a': sel}, np.ones(len(sel), dtype=bool), bins=8.0)


def sample_board(img):
    flat = clusters(img, img['flat'], 300)
    light = [c for c in flat if c[2] >= 86 and c[3] <= 14]
    dark = [c for c in flat if c[2] <= 30 and c[3] <= 22]
    res = {'canvasBg': hexs(img['canvas'])}
    lt, dt = tiers(light, 1.6), tiers(sorted(dark, key=lambda c: c[2]), 2.4)
    for role, col in zip(('surface', 'surface2', 'surface3'), lt):
        res[role] = hexs(col[0]) if col else None
    for role, col in zip(('surfaceDark', 'surfaceDark2', 'surfaceDark3'), dt):
        res[role] = hexs(col[0]) if col else None
    tint = [c for c in flat if 91 <= c[2] <= 97 and 4 <= c[3] <= 15 and 240 <= c[4] <= 330]
    soft = [c for c in flat if 78 <= c[2] <= 91 and 14 <= c[3] <= 32 and 240 <= c[4] <= 330]
    res['hoverTint'] = hexs(tint[0][0]) if tint else None
    res['accentSoft'] = hexs(soft[0][0]) if soft else None
    soft_d = [c for c in dark if 24 <= c[2] <= 45 and 18 <= c[3] <= 60 and 240 <= c[4] <= 330]
    res['accentSoftDark'] = hexs(soft_d[0][0]) if soft_d else None
    for role, bg_rng, rng, darkest in (('text', (86, 100), (0, 50), True),
                                       ('muted', (86, 100), (50, 80), True),
                                       ('line', (93, 100), (83, 95), True),
                                       ('textDark', (0, 32), (70, 100), False),
                                       ('mutedDark', (0, 32), (40, 70), False),
                                       ('lineDark', (0, 32), (28, 44), False)):
        col, _ = ink(img, bg_rng, rng, darkest)
        res[role] = hexs(col) if col is not None else None
    vio, _ = status(img, 250, 316, 0.55, 0.35)
    lite, _ = status(img, 250, 316, 0.45, 0.62)
    res['accent'] = hexs(vio) if vio is not None else None
    res['accent2'] = hexs(lite) if lite is not None else None
    for role, (lo, hi, ms, mv) in {'online': (85, 168, 0.50, 0.35),
                                   'dndAmber': (30, 66, 0.62, 0.62),
                                   'danger': (345, 372, 0.60, 0.55)}.items():
        col, n = status(img, lo, hi, ms, mv)
        res[role] = hexs(col) if col is not None else None
        res[role + 'Px'] = n
    return res


def hexs(rgb):
    if rgb is None:
        return None
    return '#%02X%02X%02X' % tuple(int(round(v)) for v in np.clip(rgb, 0, 255))


def consensus(per_board):
    """Cross-board median in Lab, with the spread a human needs to judge confidence."""
    out = {}
    keys = sorted({k for b in per_board.values() for k in b if not k.endswith('Px')})
    for role in keys:
        vals = [np.asarray(hexrgb(b[role]), dtype=float) for b in per_board.values() if b.get(role)]
        labs = rgb2lab(np.array(vals)) if len(vals) else np.empty((0, 3))
        if len(labs) < 3:
            out[role] = {'hex': None, 'boards': len(labs), 'maxDeltaE': None, 'lowConfidence': True}
            continue
        med = np.median(labs, axis=0)
        out[role] = {'hex': hexs(lab2rgb_approx(med, vals)), 'boards': int(len(labs)),
                     'maxDeltaE': round(max(de_lab(med, lab) for lab in labs), 1),
                     'lowConfidence': len(labs) < 8}
    return out


def lab2rgb_approx(lab, fallback):
    """Median Lab -> nearest sampled RGB (keeps output on real sampled pixels)."""
    best, dist = None, 1e9
    for rgb in fallback:
        d = float(np.sum((rgb2lab(rgb) - lab) ** 2))
        if d < dist:
            best, dist = rgb, d
    return np.array(best)


def hexrgb(h):
    h = h.lstrip('#')
    if len(h) == 3:
        h = ''.join(ch * 2 for ch in h)
    return [int(h[i:i + 2], 16) for i in (0, 2, 4)]


def current_tokens():
    css = CSS.read_text()
    tokens = {}
    for block, theme in ((':root{', 'light'), ('.dark{', 'dark')):
        body = css.split(block, 1)[1].split('}', 1)[0]
        for name, val in re.findall(r'(--[a-z0-9-]+):(#[0-9a-fA-F]{3,8})', body):
            tokens[f'{theme}.{name}'] = val
    return tokens


MAP = {'light.--surface': 'surface', 'light.--surface-2': 'surface2', 'light.--surface-3': 'surface3',
       'light.--text': 'text', 'light.--muted': 'muted', 'light.--line': 'line',
       'light.--accent-soft': 'hoverTint', 'light.--accent': 'accent', 'light.--accent2': 'accent2',
       'light.--green': 'online', 'light.--danger': 'danger',
       'dark.--surface': 'surfaceDark', 'dark.--surface-2': 'surfaceDark2', 'dark.--surface-3': 'surfaceDark3',
       'dark.--text': 'textDark', 'dark.--muted': 'mutedDark', 'dark.--line': 'lineDark',
       'dark.--accent-soft': 'accentSoftDark'}


def main():
    if not BOARDS:
        sys.exit('No boards found in design/*.png')
    per_board = {}
    for path in BOARDS:
        per_board[path.stem] = sample_board(load_board(path))
    cons = consensus(per_board)
    stamp = dt.datetime.fromtimestamp(max(p.stat().st_mtime for p in BOARDS), dt.timezone.utc)
    doc = {'generatedAt': stamp.replace(microsecond=0).isoformat(),
           'method': ('Pillow+numpy only. sRGB->Lab D65; CIEDE2000 for deltas. Board lavender canvas '
                      '(median of the 2px border) is excluded, then 9x9 local-std<5 selects large flat '
                      'regions for surfaces and 3x3 local-std<7 selects small UI blobs for status/accent. '
                      'Surfaces = Lab-binned clusters split into lightness tiers; text/muted/line = edge '
                      'shoulder ink cores (mode of the darkest/brightest 8% of pixels sitting on a surface '
                      'of known lightness, antialiasing excluded); consensus = per-role Lab median with '
                      'maxDeltaE spread across boards.'),
           'deltaEGate': DELTA_E_GATE,
           'boards': per_board,
           'consensus': {'light': {k: v for k, v in cons.items() if k in
                                   ('surface', 'surface2', 'surface3', 'text', 'muted', 'line', 'hoverTint', 'accentSoft')},
                         'dark': {k: v for k, v in cons.items() if k in
                                  ('surfaceDark', 'surfaceDark2', 'surfaceDark3', 'textDark', 'mutedDark', 'lineDark', 'accentSoft')},
                         'brand': {k: v for k, v in cons.items() if k in ('accent', 'accent2')},
                         'status': {k: v for k, v in cons.items() if k in ('online', 'dndAmber', 'danger')}}}
    if '--no-write' not in sys.argv:
        OUT.write_text(json.dumps(doc, indent=2) + '\n')
    print(f'{len(BOARDS)} boards -> {OUT.relative_to(ROOT)}')
    print('\nSANITY (3 regions, median of flat pixels inside the box):')
    for stem, label, box, want in SANITY_REGIONS:
        img = load_board(ROOT / 'design' / f'{stem}.png')
        x0, y0, x1, y1 = box
        crop = img['flat'][y0:y1, x0:x1]
        px = img['a'][y0:y1, x0:x1][crop]
        rgb = np.median(px, axis=0) if len(px) else img['a'][y0:y1, x0:x1].reshape(-1, 3).mean(axis=0)
        print(f'  {label:42s} {want:14s} {hexs(rgb)}  ({len(px)} flat px)')
    print('\nTOKEN AUDIT (current -> sampled, gate dE00>%.0f):' % DELTA_E_GATE)
    for token, role in MAP.items():
        cur = current_tokens().get(token)
        got = cons.get(role, {}).get('hex')
        if not cur or not got:
            print(f'  {token:22s} {cur or "-":8s} {got or "no sample":10s} -')
            continue
        d = delta_e(hexrgb(cur), hexrgb(got))
        print(f'  {token:22s} {cur:8s} {got:10s} dE={d:5.1f} {"CHANGE" if d > DELTA_E_GATE else "keep"}'
              f'  (n={cons[role]["boards"]}, spread={cons[role]["maxDeltaE"]})')


if __name__ == '__main__':
    main()
