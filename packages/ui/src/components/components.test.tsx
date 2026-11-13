import { describe, expect, it } from 'vitest';
import packageJson from '../../../../package.json';
import { placeContextMenu } from '../menu';
import {
  CHORD_MODIFIERS,
  NON_TRIGGER_KEYS,
  chordFromEvent,
  displayChord,
  formatChord,
  isCompleteChord,
  isModifierKey,
  parseChord,
  platformChord,
  triggerKeyFor,
} from './HotkeyRecorder';
import {
  THEME_OPTIONS,
  themedOptions,
  themeChoiceFor,
  themeTabIndex,
} from './ThemeCards';
import {
  WEEKDAY_DAYS,
  isEveryDay,
  isWeekdaysOnly,
  orderDays,
  toggleDay,
  weekendDays,
} from './WeekdayPicker';
import {
  APP_VERSION,
  CATEGORY_ICONS,
  CATEGORY_ORDER,
  adapterOptions,
  clockLabel,
  cornerValueFor,
  dateLabel,
  deviceOptions,
  fileNameFor,
  hotkeyDisplay,
  hotkeyForPlatform,
  hourOptions,
  numberOptions,
  paletteColorFor,
  percentLabel,
  secondsLabel,
  withCurrent,
} from '../screens/SettingsScreen';
import {
  AVATAR_PALETTE,
  MAX_COMPOSITE_MEMBERS,
  MENU_WIDTH,
  type AvatarMember,
  clampPercent,
  compositeMembers,
  deliveryTicks,
  fileKindFor,
  fileTransferDetail,
  fileTransferLabel,
  formatCount,
  formatDuration,
  formatFileSize,
  groupReactions,
  initialsFor,
  measureMenuHeight,
  nextIndexOf,
  avatarPaletteFor,
  placeMenu,
  reactionTooltip,
  transferPercent,
  typeaheadIndex,
  waveformBars,
  waveformProgress,
} from './logic';

/* ------------------------------------------------------------------ *
 * vitest runs with `environment: 'node'` (vite.config.ts), so this
 * suite covers the exported pure logic only — no React rendering.
 * ------------------------------------------------------------------ */

describe('placeMenu edge flip', () => {
  it('agrees with placeContextMenu for the pinned viewport case', () => {
    expect(placeMenu(900, 700, 1024, 768)).toEqual({ left: 796, top: 364, flipped: true });
  });

  it('leaves the menu on the pointer when it fits', () => {
    expect(placeMenu(100, 100, 1024, 768)).toEqual({ left: 100, top: 100, flipped: false });
  });

  it('flips horizontally only when it crosses the right edge', () => {
    const placed = placeMenu(1000, 50, 1024, 768);
    expect(placed).toEqual({ left: 796, top: 50, flipped: true });
    expect(placed.top).toBe(50);
  });

  it('flips vertically only when it crosses the bottom edge', () => {
    expect(placeMenu(60, 700, 1024, 768, MENU_WIDTH, 300)).toEqual({
      left: 60,
      top: 460,
      flipped: true,
    });
  });

  it('clamps to the edge margin for negative coordinates', () => {
    expect(placeMenu(-40, -80, 1024, 768)).toEqual({ left: 8, top: 8, flipped: true });
  });

  it('never returns the same left/top as the pointer once clamped', () => {
    const cases: Array<[number, number, number, number]> = [
      [900, 700, 1024, 768],
      [0, 0, 1024, 768],
      [512, 384, 1024, 768],
      [370, 650, 400, 680],
    ];
    for (const [x, y, vw, vh] of cases) {
      const mine = placeMenu(x, y, vw, vh, MENU_WIDTH, 300);
      const shared = placeContextMenu(x, y, vw, vh, MENU_WIDTH, 300);
      expect({ left: mine.left, top: mine.top }).toEqual(shared);
      expect(mine.flipped).toBe(shared.left !== x || shared.top !== y);
      expect(mine.left).toBeGreaterThanOrEqual(8);
      expect(mine.top).toBeGreaterThanOrEqual(8);
      expect(mine.left + MENU_WIDTH).toBeLessThanOrEqual(vw);
    }
  });

  it('estimates the height it feeds into the flip', () => {
    expect(measureMenuHeight(0)).toBe(12);
    expect(measureMenuHeight(4)).toBe(172);
    expect(measureMenuHeight(4, { separators: 1 })).toBe(185);
    expect(measureMenuHeight(4, { reactionRow: true })).toBe(210);
    expect(placeMenu(100, 100, 1024, 768, MENU_WIDTH, measureMenuHeight(4)).top).toBe(100);
  });
});

describe('avatar initials and palette', () => {
  it('takes the first letter of the first two words', () => {
    expect(initialsFor('Maria Ahmed')).toBe('MA');
    expect(initialsFor('arjun kumar')).toBe('AK');
    expect(initialsFor('  Layla   Noor  ')).toBe('LN');
    expect(initialsFor('Farah Pasha Siddiqui')).toBe('FP');
  });

  it('falls back to one letter, then to a question mark', () => {
    expect(initialsFor('Sarah')).toBe('S');
    expect(initialsFor('')).toBe('?');
    expect(initialsFor('   ')).toBe('?');
    expect(initialsFor(undefined)).toBe('?');
    expect(initialsFor(null)).toBe('?');
  });

  it('maps the board examples onto the eight palette slots', () => {
    expect(avatarPaletteFor('Maria Ahmed')).toBe('rose');
    expect(avatarPaletteFor('Arjun Kumar')).toBe('violet');
    expect(avatarPaletteFor('Farah Pasha')).toBe('blue');
    expect(avatarPaletteFor('Sarah Lee')).toBe('cyan');
    expect(avatarPaletteFor('Devika Rao')).toBe('violet');
    expect(avatarPaletteFor('Ravi Menon')).toBe('yellow');
    expect(avatarPaletteFor('Sanjay Kumar')).toBe('blue');
    expect(avatarPaletteFor('Nadia Latif')).toBe('green');
  });

  it('is deterministic, case/space insensitive and always in the palette', () => {
    expect(avatarPaletteFor('')).toBe('violet');
    expect(avatarPaletteFor(null)).toBe('violet');
    for (const seed of ['ana', 'Ana', '  ana  ', 'ANA', 'zoe', '12']) {
      const picked = avatarPaletteFor(seed);
      expect(AVATAR_PALETTE).toContain(picked);
      expect(picked).toBe(avatarPaletteFor(seed));
    }
    expect(avatarPaletteFor('Ana')).toBe(avatarPaletteFor('ana'));
    expect(new Set(['ana', 'BO', 'kai', 'li', 'mo', 'xi', 'yu', 'zo'].map(avatarPaletteFor)).size)
      .toBeGreaterThan(1);
  });

  it('exposes exactly the eight colours from the profile board', () => {
    expect(AVATAR_PALETTE).toHaveLength(8);
    expect(new Set(AVATAR_PALETTE).size).toBe(8);
  });
});

describe('composite group avatar members', () => {
  const members: AvatarMember[] = [
    { name: 'Offline One', presence: 'offline' },
    { name: 'Online Two', presence: 'online' },
    { name: 'Busy Three', presence: 'busy' },
    { name: 'Away Four', presence: 'away' },
    { name: 'Online Five', presence: 'online' },
    { name: 'No Presence' },
  ];

  it('slices to four members maximum', () => {
    const picked = compositeMembers(members);
    expect(picked).toHaveLength(MAX_COMPOSITE_MEMBERS);
    expect(picked.length).toBeLessThanOrEqual(4);
  });

  it('prefers online, then away, then busy, then offline', () => {
    expect(compositeMembers(members).map((member) => member.name)).toEqual([
      'Online Two',
      'Online Five',
      'Away Four',
      'Busy Three',
    ]);
  });

  it('keeps the caller order inside a presence tier', () => {
    const sameTier: AvatarMember[] = [
      { name: 'a', presence: 'online' },
      { name: 'b', presence: 'online' },
      { name: 'c', presence: 'online' },
    ];
    expect(compositeMembers(sameTier).map((member) => member.name)).toEqual(['a', 'b', 'c']);
  });

  it('treats a missing presence as offline and honours the limit', () => {
    expect(compositeMembers([{ name: 'quiet' }, { name: 'live', presence: 'online' }])).toEqual([
      { name: 'live', presence: 'online' },
      { name: 'quiet' },
    ]);
    expect(compositeMembers(members, 2)).toHaveLength(2);
    expect(compositeMembers(members, 0)).toEqual([]);
    expect(compositeMembers(members, -1)).toEqual([]);
    expect(compositeMembers([])).toEqual([]);
  });
});

describe('bubble delivery ticks', () => {
  it('draws nothing on incoming messages', () => {
    for (const delivery of ['sending', 'delivered', 'read'] as const) {
      expect(deliveryTicks(delivery, false)).toEqual({ icon: null, label: '', count: 0 });
    }
  });

  it('maps sending to a spinner, delivered to one check, read to two', () => {
    expect(deliveryTicks('sending', true)).toEqual({ icon: 'loader', label: 'Sending', count: 0 });
    expect(deliveryTicks('delivered', true)).toEqual({ icon: 'check', label: 'Delivered', count: 1 });
    expect(deliveryTicks('read', true)).toEqual({
      icon: 'checkCheck',
      label: 'Read',
      count: 2,
    });
  });

  it('falls back to the sending spinner for an unknown state', () => {
    expect(deliveryTicks(undefined, true)).toEqual({ icon: 'loader', label: 'Sending', count: 0 });
  });
});

describe('reaction grouping', () => {
  it('collapses repeats of one emoji into a single chip', () => {
    const groups = groupReactions([
      { emoji: '👍', by: 'Liyana' },
      { emoji: '❤️', by: 'Arjun' },
      { emoji: '👍', by: 'Sarah' },
      { emoji: '👍', by: 'Liyana' },
    ]);
    expect(groups).toEqual([
      { emoji: '👍', count: 2, names: ['Liyana', 'Sarah'] },
      { emoji: '❤️', count: 1, names: ['Arjun'] },
    ]);
  });

  it('keeps first-seen emoji order and de-duplicates people', () => {
    const groups = groupReactions([
      { emoji: '😂', by: 'a' },
      { emoji: '🔥', by: 'b' },
      { emoji: '😂', by: 'a' },
      { emoji: '😂', by: 'c' },
      { emoji: '🔥', by: 'b' },
    ]);
    expect(groups.map((group) => group.emoji)).toEqual(['😂', '🔥']);
    expect(groups[0]?.names).toEqual(['a', 'c']);
    expect(groups.map((group) => group.count)).toEqual([2, 1]);
  });

  it('handles an empty board', () => {
    expect(groupReactions([])).toEqual([]);
  });

  it('describes who reacted in the hover tooltip', () => {
    expect(reactionTooltip([])).toBe('');
    expect(reactionTooltip(['Liyana'])).toBe('Liyana');
    expect(reactionTooltip(['Liyana', 'Arjun', 'Sarah'])).toBe('Liyana, Arjun, Sarah');
    expect(reactionTooltip(['Liyana', 'Arjun', 'Sarah', 'Devika'])).toBe(
      'Liyana, Arjun, Sarah +1',
    );
    expect(reactionTooltip(['a', 'b', 'c', 'd', 'e', 'f'], 2)).toBe('a, b +4');
  });
});

describe('file bubble logic', () => {
  it('formats sizes, durations and transfer percentages', () => {
    expect(formatFileSize(0)).toBe('0 B');
    expect(formatFileSize(512)).toBe('512 B');
    expect(formatFileSize(2048)).toBe('2 KB');
    expect(formatFileSize(2_516_582)).toBe('2.4 MB');
    expect(formatFileSize(12_435_425)).toBe('11.9 MB');
    expect(formatFileSize(-5)).toBe('0 B');
    expect(formatDuration(0)).toBe('00:00');
    expect(formatDuration(156)).toBe('02:36');
    expect(formatDuration(3733)).toBe('01:02:13');
    expect(transferPercent(6_815_744, 12_435_425)).toBe(55);
    expect(transferPercent(10, 0)).toBe(0);
    expect(clampPercent(140)).toBe(100);
    expect(clampPercent(-4)).toBe(0);
    expect(fileTransferDetail(6_815_744, 12_435_425)).toBe('6.5 MB of 11.9 MB (55%)');
  });

  it('labels the four transfer states from the board', () => {
    expect(fileTransferLabel('paused')).toBe('Paused');
    expect(fileTransferLabel('failed')).toBe('Failed');
    expect(fileTransferLabel('complete')).toBe('Complete');
    expect(fileTransferLabel('sending')).toBe('Sending…');
    expect(fileTransferLabel(undefined, 1, 2)).toBe('Sending… 50%');
    expect(fileTransferLabel('sending', 6_815_744, 12_435_425)).toBe('Sending… 55%');
  });

  it('picks the file glyph from the extension', () => {
    expect(fileKindFor('report.pdf')).toBe('pdf');
    expect(fileKindFor('BUDGET.XLSX')).toBe('sheet');
    expect(fileKindFor('notes.md')).toBe('doc');
    expect(fileKindFor('clip.MP4')).toBe('media');
    expect(fileKindFor('voice-note.mp3')).toBe('audio');
    expect(fileKindFor('backup.tar.gz')).toBe('archive');
    expect(fileKindFor('LICENSE')).toBe('file');
    expect(fileKindFor('archive.')).toBe('file');
  });
});

describe('waveform and badge helpers', () => {
  it('produces a stable, in-range bar shape per seed', () => {
    const first = waveformBars('voice-42', 24);
    expect(first).toHaveLength(24);
    expect(first).toEqual(waveformBars('voice-42', 24));
    expect(waveformBars('voice-42', 24)).not.toEqual(waveformBars('voice-43', 24));
    for (const value of first) {
      expect(value).toBeGreaterThanOrEqual(0.12);
      expect(value).toBeLessThanOrEqual(1);
    }
    expect(waveformBars(7, 0)).toHaveLength(1);
  });

  it('tracks playback position as a 0..1 fraction', () => {
    expect(waveformProgress(0, 32)).toBe(0);
    expect(waveformProgress(8, 32)).toBe(0.25);
    expect(waveformProgress(40, 32)).toBe(1);
    expect(waveformProgress(-5, 32)).toBe(0);
    expect(waveformProgress(5, 0)).toBe(0);
  });

  it('caps badge counts', () => {
    expect(formatCount(0)).toBe('0');
    expect(formatCount(-3)).toBe('0');
    expect(formatCount(7)).toBe('7');
    expect(formatCount(99)).toBe('99');
    expect(formatCount(128)).toBe('99+');
    expect(formatCount(128, 9)).toBe('9+');
  });
});

describe('roving focus helpers', () => {
  it('moves the highlight with optional wrapping', () => {
    expect(nextIndexOf(0, -1, 3)).toBe(2);
    expect(nextIndexOf(2, 1, 3)).toBe(0);
    expect(nextIndexOf(2, 1, 3, { wrap: false })).toBe(2);
    expect(nextIndexOf(0, -1, 3, { wrap: false })).toBe(0);
    expect(nextIndexOf(1, 0, 0)).toBe(-1);
  });

  it('finds the typeahead target from the current position', () => {
    const labels = ['General', 'Privacy', 'Storage', 'About'];
    expect(typeaheadIndex(labels, 's')).toBe(2);
    expect(typeaheadIndex(labels, ' S')).toBe(2);
    expect(typeaheadIndex(labels, 'g', 1)).toBe(0);
    expect(typeaheadIndex(labels, 'zz')).toBe(-1);
    expect(typeaheadIndex(labels, '  ')).toBe(-1);
    expect(typeaheadIndex([], 'a')).toBe(-1);
  });
});

/* ================================================================== *
 * Settings-screen controls: ThemeCards, WeekdayPicker, HotkeyRecorder
 * plus the pure helpers SettingsScreen exports.
 * ================================================================== */

describe('ThemeCards logic', () => {
  it('keeps the board order Light, Dark, System', () => {
    expect(THEME_OPTIONS.map((option) => option.value)).toEqual(['light', 'dark', 'system']);
    expect(THEME_OPTIONS.map((option) => option.label)).toEqual(['Light', 'Dark', 'System']);
  });

  it('applies caller labels without mutating the shared option list', () => {
    const labels = themedOptions({ dark: 'Night' });
    expect(labels.map((option) => option.label)).toEqual(['Light', 'Night', 'System']);
    expect(labels.map((option) => option.value)).toEqual(['light', 'dark', 'system']);
    expect(THEME_OPTIONS[1]?.label).toBe('Dark');
    expect(themedOptions()).toEqual([...THEME_OPTIONS]);
    expect(themedOptions({})).toEqual([...THEME_OPTIONS]);
  });

  it('narrows any stored string onto the three real theme values', () => {
    expect(themeChoiceFor('light')).toBe('light');
    expect(themeChoiceFor('dark')).toBe('dark');
    for (const value of ['system', 'gold', '', 'AUTO', undefined, null]) {
      expect(themeChoiceFor(value as string | null | undefined)).toBe('system');
    }
  });

  it('gives exactly one card a roving tabindex of 0', () => {
    expect(themeTabIndex('light', THEME_OPTIONS)).toBe(0);
    expect(themeTabIndex('dark', THEME_OPTIONS)).toBe(1);
    expect(themeTabIndex('system', THEME_OPTIONS)).toBe(2);
    // unknown value still focuses the first card, never nothing
    expect(themeTabIndex('nope' as never, THEME_OPTIONS)).toBe(0);
    const indexes = THEME_OPTIONS.map((_, index) =>
      index === themeTabIndex('dark', THEME_OPTIONS) ? 0 : -1,
    );
    expect(indexes.filter((value) => value === 0)).toHaveLength(1);
  });
});

describe('WeekdayPicker logic', () => {
  it('offers a Monday-first week, matching the Rust `%a` strings', () => {
    expect(WEEKDAY_DAYS).toEqual(['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun']);
    expect(WEEKDAY_DAYS).toHaveLength(7);
  });

  it('splits the weekend out for the presets', () => {
    expect(weekendDays()).toEqual(['Sat', 'Sun']);
    expect(weekendDays(['Mon', 'Sun'])).toEqual(['Sun']);
    expect(weekendDays(['Mon', 'Tue'])).toEqual([]);
  });

  it('orders any selection back into week order', () => {
    expect(orderDays(['Sun', 'Fri', 'Mon'])).toEqual(['Mon', 'Fri', 'Sun']);
    expect(orderDays(['Wed', 'Wed'])).toEqual(['Wed', 'Wed']);
    expect(orderDays([])).toEqual([]);
    // unknown days keep their relative order and land last
    expect(orderDays(['Zzz', 'Tue', 'Aaa'])).toEqual(['Tue', 'Zzz', 'Aaa']);
  });

  it('toggles one day and always returns a fresh, ordered copy', () => {
    const selected = ['Mon', 'Tue', 'Wed', 'Thu', 'Fri'];
    expect(toggleDay('Sat', selected)).toEqual(['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat']);
    expect(toggleDay('Mon', selected)).toEqual(['Tue', 'Wed', 'Thu', 'Fri']);
    expect(toggleDay('Mon', toggleDay('Mon', selected))).toEqual(selected);
    const source = ['Fri', 'Mon'];
    expect(toggleDay('Wed', source)).toEqual(['Mon', 'Wed', 'Fri']);
    expect(source).toEqual(['Fri', 'Mon']);
  });

  it('detects the two presets the board offers', () => {
    expect(isEveryDay(WEEKDAY_DAYS)).toBe(true);
    expect(isEveryDay([...WEEKDAY_DAYS, 'Extra'])).toBe(true);
    expect(isEveryDay(['Mon'])).toBe(false);
    expect(isEveryDay([])).toBe(false);
    expect(isEveryDay([], [])).toBe(false);

    expect(isWeekdaysOnly(['Mon', 'Tue', 'Wed', 'Thu', 'Fri'])).toBe(true);
    expect(isWeekdaysOnly(['Fri', 'Wed', 'Mon', 'Thu', 'Tue'])).toBe(true);
    expect(isWeekdaysOnly(['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'])).toBe(false);
    expect(isWeekdaysOnly(['Mon', 'Tue', 'Wed', 'Thu'])).toBe(false);
    expect(isWeekdaysOnly(['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Xyz'])).toBe(false);
    expect(isWeekdaysOnly([])).toBe(false);
    expect(isWeekdaysOnly(['Mon'], ['Mon'])).toBe(true);
  });
});

describe('HotkeyRecorder chord parsing', () => {
  it('splits a combo into canonical modifiers plus one trigger key', () => {
    expect(CHORD_MODIFIERS).toEqual(['CmdOrCtrl', 'Alt', 'Shift']);
    expect(parseChord('CmdOrCtrl+Alt+S')).toEqual({ modifiers: ['CmdOrCtrl', 'Alt'], key: 'S' });
    expect(parseChord('Command+Alt+s')).toEqual({ modifiers: ['CmdOrCtrl', 'Alt'], key: 'S' });
    expect(parseChord('Ctrl+Alt+S')).toEqual({ modifiers: ['CmdOrCtrl', 'Alt'], key: 'S' });
    expect(parseChord('Option+Shift+K')).toEqual({ modifiers: ['Alt', 'Shift'], key: 'K' });
    expect(parseChord('super+space')).toEqual({ modifiers: ['CmdOrCtrl'], key: 'Space' });
    expect(parseChord('Space')).toEqual({ modifiers: [], key: 'Space' });
    expect(parseChord('  CmdOrCtrl + Alt + E ')).toEqual({ modifiers: ['CmdOrCtrl', 'Alt'], key: 'E' });
    expect(parseChord('')).toEqual({ modifiers: [], key: '' });
    expect(parseChord('+++')).toEqual({ modifiers: [], key: '' });
  });

  it('collapses repeated and aliased modifiers into one canonical order', () => {
    expect(parseChord('Shift+Alt+CmdOrCtrl+Alt+Ctrl+C')).toEqual({
      modifiers: ['CmdOrCtrl', 'Alt', 'Shift'],
      key: 'C',
    });
    expect(formatChord({ modifiers: ['Shift', 'Alt', 'CmdOrCtrl'], key: 'V' })).toBe(
      'CmdOrCtrl+Alt+Shift+V',
    );
    expect(formatChord({ modifiers: [], key: 'M' })).toBe('M');
    expect(formatChord({ modifiers: ['CmdOrCtrl'], key: '' })).toBe('');
  });

  it('round-trips every spelling through the canonical display form', () => {
    for (const combo of ['CmdOrCtrl+Alt+S', 'Command+Alt+S', 'Ctrl+Alt+S', 'cmd+option+s']) {
      expect(displayChord(combo)).toBe('CmdOrCtrl+Alt+S');
    }
    expect(displayChord('CmdOrCtrl+Alt+S')).toBe('CmdOrCtrl+Alt+S');
    expect(displayChord(displayChord('Shift+A'))).toBe(displayChord('Shift+A'));
    expect(displayChord('')).toBe('');
    expect(displayChord('Alt')).toBe('');
  });

  it('refuses modifier-only and navigation keys as trigger keys', () => {
    expect(isModifierKey('Meta')).toBe(true);
    expect(isModifierKey('Control')).toBe(true);
    expect(isModifierKey('S')).toBe(false);
    for (const key of NON_TRIGGER_KEYS) expect(triggerKeyFor(key)).toBeNull();
    expect(NON_TRIGGER_KEYS).toContain('Tab');
    expect(triggerKeyFor('s')).toBe('S');
    expect(triggerKeyFor(' ')).toBe('Space');
    expect(triggerKeyFor('F11')).toBe('F11');
    expect(triggerKeyFor('ArrowUp')).toBeNull();
  });

  it('builds a chord from a key event and stays silent for modifiers', () => {
    expect(
      chordFromEvent({ key: 's', ctrlKey: false, metaKey: true, altKey: true, shiftKey: false }),
    ).toEqual({ modifiers: ['CmdOrCtrl', 'Alt'], key: 'S' });
    expect(
      chordFromEvent({ key: 'Control', ctrlKey: true, metaKey: false, altKey: false, shiftKey: false }),
    ).toBeNull();
    expect(
      chordFromEvent({ key: 'k', ctrlKey: true, metaKey: false, altKey: true, shiftKey: true }),
    ).toEqual({ modifiers: ['CmdOrCtrl', 'Alt', 'Shift'], key: 'K' });
    // meta and ctrl both fold into the cross-platform token
    expect(
      chordFromEvent({ key: 'e', ctrlKey: true, metaKey: true, altKey: false, shiftKey: false }),
    ).toEqual({ modifiers: ['CmdOrCtrl'], key: 'E' });
    expect(formatChord(chordFromEvent({ key: ' ', ctrlKey: true, metaKey: false, altKey: true, shiftKey: false })!)).toBe(
      'CmdOrCtrl+Alt+Space',
    );
  });

  it('resolves CmdOrCtrl per platform and stays parseable by the Rust side', () => {
    expect(platformChord('CmdOrCtrl+Alt+S', true)).toBe('Command+Alt+S');
    expect(platformChord('CmdOrCtrl+Alt+S', false)).toBe('Ctrl+Alt+S');
    expect(platformChord('Ctrl+Alt+S', true)).toBe('Command+Alt+S');
    expect(platformChord('Alt+Shift+A', true)).toBe('Alt+Shift+A');
    expect(platformChord('', true)).toBe('');
    // the keys apps/desktop parse_hotkey() accepts, both platforms
    for (const key of ['S', 'D', 'C', 'V', 'K', 'A', 'E', 'M', 'Space']) {
      const combo = `CmdOrCtrl+Alt+${key}`;
      expect(platformChord(combo, true)).toBe(`Command+Alt+${key}`);
      expect(platformChord(combo, false)).toBe(`Ctrl+Alt+${key}`);
      expect(displayChord(platformChord(combo, true))).toBe(combo);
      expect(displayChord(platformChord(combo, false))).toBe(combo);
    }
  });

  it('only calls a combo complete with a modifier and a key', () => {
    expect(isCompleteChord('CmdOrCtrl+Alt+S')).toBe(true);
    expect(isCompleteChord('Shift+Space')).toBe(true);
    for (const combo of ['', 'S', 'Shift', 'CmdOrCtrl', 'Alt+Shift']) {
      expect(isCompleteChord(combo)).toBe(false);
    }
  });
});

describe('SettingsScreen helpers', () => {
  it('exposes the seven board categories with an icon each', () => {
    expect(CATEGORY_ORDER).toEqual([
      'general',
      'icon',
      'notifications',
      'messages',
      'calls',
      'privacy',
      'about',
    ]);
    expect(Object.keys(CATEGORY_ICONS).sort()).toEqual([...CATEGORY_ORDER].sort());
    // The about panel reads this constant, so it has to follow the release version.
    expect(APP_VERSION).toBe(packageJson.version);
  });

  it('formats slider read-outs', () => {
    expect(secondsLabel(5)).toBe('5 s');
    expect(secondsLabel(0)).toBe('0 s');
    expect(percentLabel(90)).toBe('90%');
    expect(percentLabel(100)).toBe('100%');
  });

  it('keeps only real corner values and falls back to the Rust default', () => {
    for (const corner of [
      'left-top',
      'left-center',
      'left-bottom',
      'right-top',
      'right-center',
      'right-bottom',
    ]) {
      expect(cornerValueFor(corner)).toBe(corner);
    }
    expect(cornerValueFor('top')).toBe('right-center');
    expect(cornerValueFor('')).toBe('right-center');
  });

  it('passes unparsable clock values through untouched and formats real ones', () => {
    for (const bad of ['24:00', '07:60', 'abc', '07', '']) {
      expect(clockLabel(bad)).toBe(bad);
    }
    expect(clockLabel('13:30:00')).toContain('30');
    expect(clockLabel('07:00')).toContain('00');
    expect(hourOptions()).toHaveLength(24);
    expect(hourOptions().map((option) => option.value)).toContain('00:00');
    expect(hourOptions()[23]?.value).toBe('23:00');
    for (const option of hourOptions()) {
      expect(/^\d{2}:00$/.test(option.value)).toBe(true);
      expect(option.label.length).toBeGreaterThan(0);
    }
    expect(dateLabel(0)).toContain('1970');
  });

  it('turns numbers into dropdown choices', () => {
    expect(numberOptions([0, 1, 5], (value) => `${value} min`)).toEqual([
      { value: '0', label: '0 min' },
      { value: '1', label: '1 min' },
      { value: '5', label: '5 min' },
    ]);
    expect(numberOptions([], String)).toEqual([]);
  });

  it('lists devices behind a Default entry and keeps unknown ids selectable', () => {
    const devices = [
      { deviceId: 'mic-1', kind: 'audioinput', label: ' MacBook Mic ' },
      { deviceId: 'spk-1', kind: 'audiooutput', label: 'Speakers' },
      { deviceId: '', kind: 'audioinput', label: 'No id' },
      { deviceId: 'anon', kind: 'audioinput', label: '   ' },
      { deviceId: 'cam', kind: 'videoinput', label: 'FaceTime' },
    ] as unknown as MediaDeviceInfo[];
    expect(deviceOptions(devices, 'audioinput', 'Default microphone')).toEqual([
      { value: 'default', label: 'Default microphone' },
      { value: 'mic-1', label: ' MacBook Mic ' },
    ]);
    expect(deviceOptions(devices, 'videoinput', 'Default camera')).toHaveLength(2);
    expect(deviceOptions([], 'audiooutput', 'Default speakers')).toEqual([
      { value: 'default', label: 'Default speakers' },
    ]);

    const base = [{ value: 'mic-1', label: 'MacBook Mic' }];
    expect(withCurrent(base, 'mic-1', 'x')).toEqual(base);
    expect(withCurrent(base, 'default', 'x')).toEqual(base);
    expect(withCurrent(base, '', 'x')).toEqual(base);
    expect(withCurrent(base, 'mic-9', 'USB Headset')).toEqual([
      ...base,
      { value: 'mic-9', label: 'USB Headset' },
    ]);
    const source = [{ value: 'a', label: 'A' }];
    const copy = withCurrent(source, 'a', 'A');
    copy.push({ value: 'b', label: 'B' });
    expect(source).toHaveLength(1);
  });

  it('always offers the automatic adapter first', () => {
    expect(adapterOptions(['Wi-Fi (192.168.1.88)'], 'Automatic')).toEqual([
      { value: 'auto', label: 'Automatic' },
      { value: 'Wi-Fi (192.168.1.88)', label: 'Wi-Fi (192.168.1.88)' },
    ]);
    expect(adapterOptions([], 'Automatic')).toHaveLength(1);
  });

  it('reads the last path segment on either separator', () => {
    expect(fileNameFor('/Users/anshad/Backups/alverchat.tar')).toBe('alverchat.tar');
    expect(fileNameFor('C:\\Users\\anshad\\backup.zip')).toBe('backup.zip');
    expect(fileNameFor('backup.zip')).toBe('backup.zip');
    expect(fileNameFor('')).toBe('');
    expect(fileNameFor('a/b/')).toBe('b');
  });

  it('maps avatar colours onto the palette only', () => {
    for (const color of AVATAR_PALETTE) expect(paletteColorFor(color)).toBe(color);
    expect(paletteColorFor('magenta')).toBeUndefined();
    expect(paletteColorFor(undefined)).toBeUndefined();
    expect(paletteColorFor('')).toBeUndefined();
  });

  it('displays chords canonically but stores them per platform', () => {
    expect(hotkeyDisplay('Command+Alt+S')).toBe('CmdOrCtrl+Alt+S');
    expect(hotkeyDisplay('Ctrl+Alt+S')).toBe('CmdOrCtrl+Alt+S');
    expect(hotkeyDisplay('')).toBe('');
    expect(hotkeyForPlatform('CmdOrCtrl+Alt+S', true)).toBe('Command+Alt+S');
    expect(hotkeyForPlatform('CmdOrCtrl+Alt+S', false)).toBe('Ctrl+Alt+S');
  });
});
