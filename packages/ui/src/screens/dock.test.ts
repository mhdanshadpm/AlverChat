import { describe, expect, it } from 'vitest';
import {
  callClockLabel,
  dockIconMood,
  dockOpacity,
  dockTooltip,
  idlePhotoFor,
  morphRevertMs,
  oldestFirst,
  peerTimeLabel,
  showMorphedIcon,
} from './dock';

describe('dockOpacity', () => {
  it('turns the percentage into a CSS alpha', () => {
    expect(dockOpacity(90)).toBe(0.9);
    expect(dockOpacity(100)).toBe(1);
    expect(dockOpacity(20)).toBe(0.2);
  });

  it('clamps a value outside the picker instead of hiding the dock', () => {
    // A restored backup or hand-edited settings file can hold anything; opacity 0 would
    // leave an invisible icon that still swallows the clicks under it.
    expect(dockOpacity(0)).toBe(0.2);
    expect(dockOpacity(150)).toBe(1);
    expect(dockOpacity(Number.NaN)).toBe(1);
  });
});

describe('idlePhotoFor', () => {
  const photo = 'data:image/png;base64,AAAA';

  it('uses the profile photo only for the photo choice', () => {
    expect(idlePhotoFor('photo', 0, photo)).toBe(photo);
    expect(idlePhotoFor('logo', 0, photo)).toBeNull();
  });

  it('keeps the unread avatar while a message is waiting', () => {
    expect(idlePhotoFor('photo', 2, photo)).toBeNull();
  });

  it('falls back to the logo when no photo is set', () => {
    expect(idlePhotoFor('photo', 0, null)).toBeNull();
    expect(idlePhotoFor('photo', 0, '')).toBeNull();
  });
});

describe('showMorphedIcon', () => {
  it('keeps the idle face while the morph switch is off', () => {
    expect(showMorphedIcon(false, false, false)).toBe(false);
    expect(showMorphedIcon(false, false, true)).toBe(false);
  });

  it('hands the icon back after the revert delay', () => {
    expect(showMorphedIcon(true, false, false)).toBe(true);
    expect(showMorphedIcon(true, true, false)).toBe(false);
  });

  it('does not revert the face of a call that is still happening', () => {
    expect(showMorphedIcon(true, true, true)).toBe(true);
  });
});

describe('morphRevertMs', () => {
  it('turns the slider value into a timer', () => {
    expect(morphRevertMs(5)).toBe(5000);
    expect(morphRevertMs(30)).toBe(30_000);
  });

  it('keeps a stored value inside the range the slider offers', () => {
    // The setting is a byte on disk: 0 would fire before the face is even painted and
    // 200 would leave the icon wearing a chat from three minutes ago.
    expect(morphRevertMs(0)).toBe(1000);
    expect(morphRevertMs(200)).toBe(30_000);
    expect(morphRevertMs(Number.NaN)).toBe(5000);
  });
});

describe('dockIconMood', () => {
  it('lets a live call outrank a quiet device', () => {
    // The board draws the call as its own frame, and a call you are in is the more
    // urgent thing to say — the muted icon would read as "nothing will reach you".
    expect(dockIconMood(true, true)).toBe('call');
    expect(dockIconMood(true, false)).toBe('call');
    expect(dockIconMood(false, true)).toBe('dnd');
    expect(dockIconMood(false, false)).toBe('default');
  });
});

describe('callClockLabel', () => {
  it('counts a connected call up the way the board shows it', () => {
    expect(callClockLabel(1_000, 1_000)).toBe('00:00');
    expect(callClockLabel(0, 134_000)).toBe('02:14');
    expect(callClockLabel(60_000, 65_000)).toBe('00:05');
  });

  it('never shows a negative or unbounded timer', () => {
    // A ringing call has no start time, and a clock skew could put the answer in the
    // future; both would otherwise print "-00:31" in the chip.
    expect(callClockLabel(10_000, 0)).toBe('00:00');
    expect(callClockLabel(0, 3_600_000)).toBe('60:00');
    expect(callClockLabel(0, 86_400_000)).toBe('99:59');
    expect(callClockLabel(Number.NaN, 1000)).toBe('00:00');
    expect(callClockLabel(1000, Number.POSITIVE_INFINITY)).toBe('00:00');
  });
});

describe('oldestFirst', () => {
  const peer = (name: string, lastMessageMs?: number) => ({ name, lastMessageMs });

  it('puts the newest thread last, whatever order the shell sent', () => {
    expect(oldestFirst([peer('Arjun', 300), peer('Billing', 100), peer('Fathima', 200)]).map(p => p.name)).toEqual([
      'Billing',
      'Fathima',
      'Arjun',
    ]);
  });

  it('treats an unstamped chat as the oldest', () => {
    // The browser preview has no stamp at all, so it must not land in the newest slot.
    expect(oldestFirst([peer('Arjun'), peer('Fathima', 1)]).map(p => p.name)).toEqual(['Arjun', 'Fathima']);
  });

  it('breaks a tie by name so the order is stable', () => {
    expect(oldestFirst([peer('Zoe', 5), peer('Anu', 5)]).map(p => p.name)).toEqual(['Anu', 'Zoe']);
  });

  it('leaves the caller list untouched', () => {
    const list = [peer('Arjun', 2), peer('Billing', 1)];
    expect(oldestFirst(list)).not.toBe(list);
    expect(list.map(p => p.name)).toEqual(['Arjun', 'Billing']);
  });
});

describe('peerTimeLabel', () => {
  it('formats the desktop stamp in the local clock', () => {
    const ms = new Date(2026, 10, 14, 9, 26).getTime();
    expect(peerTimeLabel({ lastMessageMs: ms })).toBe(
      new Date(ms).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' }),
    );
  });

  it('falls back to the pre-formatted preview string', () => {
    expect(peerTimeLabel({ time: '10:24' })).toBe('10:24');
    expect(peerTimeLabel({})).toBe('');
    expect(peerTimeLabel({ lastMessageMs: 0, time: '10:24' })).toBe('10:24');
  });
});

describe('dockTooltip', () => {
  it('names the person the call is with', () => {
    expect(dockTooltip({ inCall: true, peerName: 'Arjun Kumar', unreadTotal: 3, quiet: false })).toBe(
      'In call with Arjun Kumar',
    );
    // A peer who has left the network has no row to read a name from.
    expect(dockTooltip({ inCall: true, peerName: '', unreadTotal: 0, quiet: false })).toBe('In a call');
  });

  it('counts unread messages against the newest chat', () => {
    expect(dockTooltip({ inCall: false, peerName: 'Fathima P', unreadTotal: 12, quiet: false })).toBe(
      'Fathima P · 12 new messages',
    );
    // The count is one, so the word is singular: this sentence used to read "1 new messages".
    expect(dockTooltip({ inCall: false, peerName: '', unreadTotal: 1, quiet: false })).toBe('1 new message');
    expect(dockTooltip({ inCall: false, peerName: 'Fathima P', unreadTotal: 1, quiet: false })).toBe('Fathima P · 1 new message');
  });

  it('says the device is quiet instead of promising an alert', () => {
    expect(dockTooltip({ inCall: false, peerName: '', unreadTotal: 0, quiet: true })).toBe('Notifications are paused');
    expect(dockTooltip({ inCall: false, peerName: '', unreadTotal: 0, quiet: false })).toBe('AlverChat');
  });
});
