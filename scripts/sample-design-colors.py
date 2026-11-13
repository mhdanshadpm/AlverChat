"""Sample AlverChat UI colour tokens straight from the design board pixels.

    python3 scripts/sample-design-colors.py            # sample + write docs/design-colors.json
    python3 scripts/sample-design-colors.py --no-write # dry run

No network, no downloads, no RNG. "generatedAt" is the newest board mtime rather than the wall
clock, so two runs in a row give a byte-identical JSON and stdout.
"""
from collections import deque
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
DELTA_E_GATE = 6.0   # a token is only "wrong" past this CIEDE2000 distance...
SPREAD_GATE = 12.0   # ...and only when the twelve boards agree with each other

METHOD = ('Pillow+numpy only; sRGB->Lab D65 and CIEDE2000 for every delta. The board lavender canvas '
          '(median of the 2px border) is removed by a border-connected flood fill, since a colour test '
          'alone cannot tell it apart from the UI near-white surfaces. 9x9 local-std<5 picks the large '
          'flat regions used for surfaces, 3x3 local-std<7 the small blobs used for status and brand; '
          'surfaces are Lab-binned clusters split into lightness tiers. text/muted are edge-shoulder ink '
          'cores (mode of the darkest/brightest 8% of pixels lying on a surface of known lightness, so '
          'antialiasing drops out); line is a long constant run one small lightness step below the row '
          'above (drop shadows step far too much); accent/accent2 are the violet gradient stops at its '
          '0.18 and 0.82 lightness quantiles. Consensus = per-role Lab median snapped to the nearest '
          'sampled RGB, maxDeltaE = cross-board disagreement.')

# Three fixed regions a human can eyeball: (board stem, label, box, what it should be).
SANITY_REGIONS = [
    ('LAN_Chat_UI_Themes_and_States', 'light theme chat card', (120, 100, 200, 160), 'light surface'),
    ('LAN_Chat_UI_Themes_and_States', 'dark theme chat panel', (820, 300, 900, 360), 'dark surface'),
    ('AlverChat_Settings_Design_System', 'primary button fill', (1255, 968, 1330, 995), 'brand accent'),
]

STATUS_BANDS = {'online': (85, 168, 0.62, 0.42),   # hue window, min saturation, min value
                'dndAmber': (30, 55, 0.65, 0.60),
                'danger': (345, 372, 0.60, 0.55)}

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

def _de2000(l1, a1, b1, l2, a2, b2):
    """CIEDE2000 between two Lab triples."""
    ang = lambda a, b: math.degrees(math.atan2(b, a)) % 360.0
    cb = (math.hypot(a1, b1) + math.hypot(a2, b2)) / 2.0
    g = 0.5 * (1 - math.sqrt(cb ** 7 / (cb ** 7 + 25.0 ** 7))) if cb else 0.0
    a1p, a2p = (1 + g) * a1, (1 + g) * a2
    c1p, c2p, h1p, h2p = math.hypot(a1p, b1), math.hypot(a2p, b2), ang(a1p, b1), ang(a2p, b2)
    if c1p * c2p == 0:
        dh, hb = 0.0, h1p + h2p
    else:
        dh = h2p - h1p if abs(h2p - h1p) <= 180 else (h2p - h1p + 360 if h2p < h1p else h2p - h1p - 360)
        s2 = h1p + h2p
        hb = s2 / 2.0 if abs(h1p - h2p) <= 180 else ((s2 + 360) / 2.0 if s2 < 360 else (s2 - 360) / 2.0)
    dc, lb, cbp = c2p - c1p, (l1 + l2) / 2.0, (c1p + c2p) / 2.0
    dhp = 2 * math.sqrt(c1p * c2p) * math.sin(math.radians(dh / 2.0))
    t = (1 - 0.17 * math.cos(math.radians(hb - 30)) + 0.24 * math.cos(math.radians(2 * hb))
         + 0.32 * math.cos(math.radians(3 * hb + 6)) - 0.20 * math.cos(math.radians(4 * hb - 63)))
    sl = 1 + 0.015 * (lb - 50) ** 2 / math.sqrt(20 + (lb - 50) ** 2)
    sc, sh = 1 + 0.045 * cbp, 1 + 0.015 * cbp * t
    rt = -2 * math.sqrt(cbp ** 7 / (cbp ** 7 + 25.0 ** 7)) * \
        math.sin(math.radians(60 * math.exp(-((hb - 275) / 25) ** 2)))
    return math.sqrt(((l2 - l1) / sl) ** 2 + (dc / sc) ** 2 + (dhp / sh) ** 2 + rt * (dc / sc) * (dhp / sh))

def delta_e(rgb1, rgb2):
    """CIEDE2000 between two sRGB triples."""
    pair = rgb2lab(np.array([rgb1, rgb2], dtype=float))
    return _de2000(*pair[0], *pair[1])

def box_mean(a, k):
    """Mean of a (2k+1)^2 window per pixel and channel, via a separable integral image."""
    c = np.zeros((a.shape[0] + 2 * k + 1, a.shape[1] + 2 * k + 1, a.shape[2]))
    c[1:, 1:] = np.cumsum(np.cumsum(np.pad(a, ((k, k), (k, k), (0, 0)), 'edge'), 0), 1)
    hi, lo = slice(2 * k + 1, None), slice(None, -(2 * k + 1))
    return (c[hi, hi] - c[lo, hi] - c[hi, lo] + c[lo, lo]) / (2 * k + 1) ** 2

def canvas_region(a, canvas, step=8):
    """The board's own background: canvas-coloured AND connected to the image border."""
    near = np.abs(a - canvas).sum(axis=2) < 34.0
    small = near[::step, ::step]
    h, w = small.shape
    seen = np.zeros((h, w), bool)
    dq = deque({(i, j) for i in (0, h - 1) for j in range(w) if small[i, j]} |
               {(i, j) for j in (0, w - 1) for i in range(h) if small[i, j]})
    for i, j in dq:
        seen[i, j] = True
    while dq:
        i, j = dq.popleft()
        for ni, nj in ((i - 1, j), (i + 1, j), (i, j - 1), (i, j + 1)):
            if 0 <= ni < h and 0 <= nj < w and small[ni, nj] and not seen[ni, nj]:
                seen[ni, nj] = True
                dq.append((ni, nj))
    return near & np.kron(seen, np.ones((step, step), bool))[:a.shape[0], :a.shape[1]]

def load_board(path):
    a = np.asarray(Image.open(path).convert('RGB'), dtype=np.float32)
    canvas = np.median(np.concatenate([a[:2].reshape(-1, 3), a[-2:].reshape(-1, 3),
                                       a[:, :2].reshape(-1, 3), a[:, -2:].reshape(-1, 3)]), axis=0)
    lab, mean9, mean3 = rgb2lab(a), box_mean(a, 4), box_mean(a, 1)
    spread = lambda k, m: np.sqrt(np.maximum(box_mean(a * a, k) - m ** 2, 0)).mean(axis=2)
    std9, std3 = spread(4, mean9), spread(1, mean3)
    off = ~canvas_region(a, canvas)                # ignore the board's own lavender background
    return dict(a=a, canvas=canvas, off=off, std9=std9, mean_lab=rgb2lab(mean9),
                flat=off & (std9 < 5.0), smooth=off & (std3 < 7.0),
                lch=np.concatenate([lab, np.hypot(lab[..., 1], lab[..., 2])[..., None],
                                    (np.degrees(np.arctan2(lab[..., 2], lab[..., 1])) % 360)[..., None]], -1))

def clusters(rgb, min_count, bins=(5.0, 6.0, 6.0)):
    """Rank the flat colour blobs of a pixel set: [(rgb, count, L, C, hue), ...] by pixel count."""
    if not len(rgb):
        return []
    key = np.clip(np.round(rgb2lab(rgb) / np.array(bins)).astype(np.int32), -64, 63)
    code = (key[:, 0] + 64) * 16512 + (key[:, 1] + 64) * 128 + (key[:, 2] + 64)
    vals, counts = np.unique(code, return_counts=True)
    out = []
    for v, n in sorted(zip(vals.tolist(), counts.tolist()), key=lambda z: -z[1])[:80]:
        if n >= min_count:
            col = np.median(rgb[code == v], axis=0)
            l, a, b = rgb2lab(col[None])[0]
            out.append((col, n, float(l), float(math.hypot(a, b)),
                        float(math.degrees(math.atan2(b, a)) % 360)))
    return out

def peak(rgb, bins=5.0):
    """Most populous colour (Lab mode) of an already filtered pixel set."""
    if len(rgb) < 20:
        return None, 0
    key = np.round(rgb2lab(rgb) / bins).astype(np.int32)
    code = key[:, 0] * 65536 + key[:, 1] * 256 + key[:, 2]        # bin indices stay inside +-64
    vals, counts = np.unique(code, return_counts=True)
    return np.median(rgb[code == vals[np.argmax(counts)]], axis=0), int(counts.max())

def tiers(cands, gap, n=3):
    """Pick n well-separated lightness tiers from count-ranked clusters."""
    picked = []
    for c in cands:
        if all(abs(c[2] - p[2]) >= gap for p in picked):
            picked.append(c)
        if len(picked) == n:
            break
    return picked

def ink(img, bg_l, rng, darkest):
    """Text/muted: ink cores at the shoulders of a surface whose lightness we already know."""
    sur, l, c = img['mean_lab'][..., 0], img['lch'][..., 0], img['lch'][..., 3]
    sel = np.where((img['std9'] > 12) & img['off'] & (sur >= bg_l[0]) & (sur <= bg_l[1])
                   & (l >= rng[0]) & (l <= rng[1]) & (c < 30))
    if len(sel[0]) < 25:
        return None, 0
    px = img['a'][sel]
    order, cut = np.argsort(rgb2lab(px)[:, 0], kind='stable'), max(20, int(len(px) * 0.08))
    return peak(px[order[:cut]] if darkest else px[order[-cut:]])

def status(px, h, s, v, lo, hi, min_sat, min_val, max_val=0.99):
    """Saturated UI blobs (status dots, badges) inside an HSV hue window."""
    band = (h >= lo) & (h < hi) if hi <= 360 else ((h >= lo) | (h < hi - 360))
    sel = px[band & (s > min_sat) & (v > min_val) & (v < max_val)] * 255.0
    return peak(sel, 8.0) if len(sel) >= 12 else (None, 0)

def hairlines(img, bg_l, plo, phi):
    """True 1px dividers: long runs of one colour one small lightness step below the row above.
    A plain "slightly darker than the surface" test also catches card drop shadows, which sit
    far darker than the divider token, hence the run length plus the bounded step."""
    a, l = img['a'], img['lch'][..., 0]
    same = np.abs(np.diff(a, axis=1)).max(axis=2) <= 3.0
    run = np.zeros_like(same, dtype=np.int32)
    for j in range(1, same.shape[1]):
        run[:, j] = np.where(same[:, j], run[:, j - 1] + 1, 0)
    run = np.pad(run, ((0, 0), (1, 0)))[:, :a.shape[1]]            # np.diff shrank the width by 1
    step = np.abs(np.roll(l, -3, axis=0) - l)
    sur = np.clip(np.roll(img['mean_lab'][..., 0], -3, axis=0), 0, 100)
    hit = ((run >= 40) & (step > 2.0) & (step < 14.0) & img['off'] & (l >= plo) & (l <= phi)
           & (sur >= bg_l[0]) & (sur <= bg_l[1]) & (img['lch'][..., 3] < 16))
    return peak(a[hit], 4.0) if hit.sum() >= 25 else (None, 0)

def blob_hsv(img):
    """HSV of every small-flat pixel, i.e. of the status dots, badges and gradient stops."""
    px = img['a'][img['smooth']] / 255.0
    mx, mn = px.max(axis=1), px.min(axis=1)
    d = np.maximum(mx - mn, 1e-6)
    r, g, b = px[:, 0], px[:, 1], px[:, 2]
    h = np.where(mx == r, ((g - b) / d) % 6, np.where(mx == g, ((b - r) / d) + 2, ((r - g) / d) + 4)) * 60
    return px, h, d / np.maximum(mx, 1e-6), mx

def violet_ends(px, h, s, v):
    """--accent/--accent2 are the two stops of the brand gradient, not flat fills: read the mode
    near its dark and light ends, keeping only mid-lightness violets so that the dark wallpaper
    artwork is not mistaken for UI accent."""
    sel = px[(h >= 252) & (h < 316) & (s > 0.42) & (v > 0.30) & (v < 0.99)] * 255.0
    lab = rgb2lab(sel)
    keep = (lab[:, 0] >= 35) & (lab[:, 0] <= 75)
    sel, l = sel[keep], lab[keep][:, 0]
    if len(sel) < 120:
        return None, None
    ends = []
    for q in (0.18, 0.82):
        cut = np.quantile(l, q)
        band = sel[(l >= cut - 3.0) & (l <= cut + 3.0)]
        ends.append(peak(band, 6.0)[0] if len(band) >= 40 else None)
    return ends[0], ends[1]

def sample_board(img):
    flat = clusters(img['a'][img['flat']], 300)
    px, h, s, v = blob_hsv(img)
    res = {'canvasBg': hexs(img['canvas'])}
    light = sorted([c for c in flat if c[2] >= 86 and c[3] <= 14 and c[1] >= 1500], key=lambda c: -c[2])
    dark = sorted([c for c in flat if c[2] <= 30 and c[3] <= 22 and c[1] >= 1500], key=lambda c: c[2])
    for cols, names in ((tiers(light, 1.6), ('surface', 'surface2', 'surface3')),
                        (tiers(dark, 2.4), ('surfaceDark', 'surfaceDark2', 'surfaceDark3'))):
        for role, col in zip(names, cols):
            res[role] = hexs(col[0]) if col else None
    vio = [c for c in flat if 230 <= c[4] <= 330]
    for role, l, c in (('hoverTint', (90, 97.5), (3, 16)), ('accentSoft', (78, 91), (10, 32)),
                       ('accentSoftDark', (22, 45), (15, 60))):
        got = [x for x in vio if l[0] <= x[2] <= l[1] and c[0] <= x[3] <= c[1]]
        res[role] = hexs(got[0][0]) if got else None
    for role, bg, rng, deepest in (('text', (86, 100), (0, 50), True), ('muted', (86, 100), (48, 80), True),
                                   ('textDark', (0, 32), (70, 100), False),
                                   ('mutedDark', (0, 32), (40, 70), False)):
        res[role] = hexs(ink(img, bg, rng, deepest)[0])
    for role, bg, lo, hi in (('line', (92, 100), 82, 95), ('lineDark', (0, 32), 24, 52)):
        res[role] = hexs(hairlines(img, bg, lo, hi)[0])
    for role, col in zip(('accent', 'accent2'), violet_ends(px, h, s, v)):
        res[role] = hexs(col)
    for role, band in STATUS_BANDS.items():
        col, n = status(px, h, s, v, *band)
        res[role], res[role + 'Px'] = hexs(col), n
    return res

def hexs(rgb):
    return None if rgb is None else '#%02X%02X%02X' % tuple(int(round(float(x))) for x in np.clip(rgb, 0, 255))

def hexrgb(h):
    h = h.lstrip('#')
    h = ''.join(c * 2 for c in h) if len(h) == 3 else h           # #fff is a legal CSS hex
    return [int(h[i:i + 2], 16) for i in (0, 2, 4)]

def consensus(per_board):
    """Per-role Lab median snapped to a real sampled pixel, plus the cross-board disagreement."""
    out = {}
    for role in sorted({k for b in per_board.values() for k in b if not k.endswith('Px')}):
        vals = [np.asarray(hexrgb(b[role]), float) for b in per_board.values() if b.get(role)]
        labs = rgb2lab(np.array(vals)) if len(vals) else np.empty((0, 3))
        if len(labs) < 3:
            out[role] = {'hex': None, 'boards': len(labs), 'maxDeltaE': None, 'lowConfidence': True}
            continue
        med = np.median(labs, axis=0)
        spread = round(max(_de2000(*med, *lab) for lab in labs), 1)
        out[role] = {'hex': hexs(min(vals, key=lambda rgb: float(np.sum((rgb2lab(rgb) - med) ** 2)))),
                     'boards': int(len(labs)), 'maxDeltaE': spread,
                     'lowConfidence': bool(len(labs) < 6 or spread > SPREAD_GATE)}
    return out

def current_tokens():
    """Every colour custom property inside the :root and .dark blocks of styles.css."""
    css, tokens = CSS.read_text(), {}
    for block, theme in ((':root{', 'light'), ('.dark{', 'dark')):
        body = css.split(block, 1)[1].split('}', 1)[0]
        tokens.update({f'{theme}.{name}': val for name, val in
                       re.findall(r'(--[a-z0-9-]+):(#[0-9a-fA-F]{3,8})', body)})
    return tokens

MAP = {'light.--surface': 'surface', 'light.--surface-2': 'surface2', 'light.--surface-3': 'surface3',
       'light.--text': 'text', 'light.--muted': 'muted', 'light.--line': 'line',
       'light.--accent-soft': 'hoverTint', 'light.--accent': 'accent', 'light.--accent2': 'accent2',
       'light.--green': 'online', 'light.--danger': 'danger', 'dark.--surface': 'surfaceDark',
       'dark.--surface-2': 'surfaceDark2', 'dark.--surface-3': 'surfaceDark3', 'dark.--text': 'textDark',
       'dark.--muted': 'mutedDark', 'dark.--line': 'lineDark', 'dark.--accent-soft': 'accentSoftDark'}

GROUPS = {'light': ('surface', 'surface2', 'surface3', 'text', 'muted', 'line', 'hoverTint', 'accentSoft'),
          'dark': ('surfaceDark', 'surfaceDark2', 'surfaceDark3', 'textDark', 'mutedDark',
                   'lineDark', 'accentSoftDark'),
          'brand': ('accent', 'accent2'), 'status': ('online', 'dndAmber', 'danger')}

def main():
    if not BOARDS:
        sys.exit('No boards found in design/*.png')
    per_board = {p.stem: sample_board(load_board(p)) for p in BOARDS}
    cons = consensus(per_board)
    stamp = dt.datetime.fromtimestamp(max(p.stat().st_mtime for p in BOARDS), dt.timezone.utc)
    doc = {'generatedAt': stamp.replace(microsecond=0).isoformat(), 'method': METHOD,
           'deltaEGate': DELTA_E_GATE, 'boards': per_board,
           'consensus': {g: {k: cons[k] for k in roles if k in cons} for g, roles in GROUPS.items()}}
    if '--no-write' not in sys.argv:
        OUT.write_text(json.dumps(doc, indent=2) + '\n')
    print(f'{len(BOARDS)} boards -> {OUT.relative_to(ROOT)}')
    print('\nSANITY (3 regions, median of flat pixels inside the box):')
    for stem, label, box, want in SANITY_REGIONS:
        img = load_board(ROOT / 'design' / f'{stem}.png')
        x0, y0, x1, y1 = box
        crop, px = img['flat'][y0:y1, x0:x1], img['a'][y0:y1, x0:x1]
        rgb = np.median(px[crop], axis=0) if crop.sum() else px.reshape(-1, 3).mean(axis=0)
        print(f'  {label:42s} {want:14s} {hexs(rgb)}  ({int(crop.sum())} flat px)')
    print('\nTOKEN AUDIT (current -> sampled; change needs dE00>%.0f AND cross-board spread<=%.0f):'
          % (DELTA_E_GATE, SPREAD_GATE))
    tokens = current_tokens()
    for token, role in MAP.items():
        cur, got = tokens.get(token), cons.get(role, {}).get('hex')
        if not cur or not got:
            print(f'  {token:22s} {cur or "-":8s} {got or "no sample":10s} -')
            continue
        d, spread = delta_e(hexrgb(cur), hexrgb(got)), cons[role]['maxDeltaE']
        verdict = 'CHANGE' if d > DELTA_E_GATE and spread <= SPREAD_GATE else 'keep'
        why = '' if verdict == 'CHANGE' or d <= DELTA_E_GATE else ' (boards disagree)'
        print(f'  {token:22s} {cur:8s} {got:10s} dE={d:5.1f} {verdict}{why}'
              f'  (n={cons[role]["boards"]}, spread={spread})')

if __name__ == '__main__':
    main()
