import { describe, expect, it } from 'vitest';
import { ringPattern } from './ringtone';

describe('ringPattern', () => {
  it('gives every melody its own notes', () => {
    expect(ringPattern('alverchat').notes.map((note) => note.hz)).toEqual([740, 560]);
    expect(ringPattern('chime').notes.map((note) => note.hz)).toEqual([659.25, 880, 659.25]);
    expect(ringPattern('alverchat')).not.toEqual(ringPattern('chime'));
  });

  it('plays "None" as silence, not as a quiet ring', () => {
    expect(ringPattern('none').notes).toEqual([]);
    expect(ringPattern('none').repeatMs).toBe(0);
  });

  it('falls back to the default melody for a value it does not know', () => {
    // A restored backup or a renamed option must never leave the phone-side ring silent.
    expect(ringPattern('')).toEqual(ringPattern('alverchat'));
    expect(ringPattern('didgeridoo')).toEqual(ringPattern('alverchat'));
    expect(ringPattern('constructor')).toEqual(ringPattern('alverchat'));
  });

  it('starts each ring at zero and never schedules notes backwards', () => {
    for (const id of ['alverchat', 'chime']) {
      const times = ringPattern(id).notes.map((note) => note.at);
      expect(times[0]).toBe(0);
      expect(times).toEqual([...times].sort((a, b) => a - b));
      expect(ringPattern(id).repeatMs).toBeGreaterThan(times.at(-1)! * 1000);
    }
  });
});
