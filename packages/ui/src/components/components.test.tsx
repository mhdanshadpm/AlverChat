import { describe, expect, it } from 'vitest';
import { placeContextMenu } from '../menu';
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
