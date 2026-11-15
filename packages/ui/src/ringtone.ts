export type RingNote = { at: number; hz: number };
export type RingPattern = { notes: RingNote[]; repeatMs: number };

/**
 * The melodies Settings → Calls → Ringtone offers, written as WebAudio note patterns so
 * nothing has to ship as an asset. `at` is seconds from the start of one ring.
 */
const ALVERCHAT: RingPattern = {
  notes: [
    { at: 0, hz: 740 },
    { at: 0.45, hz: 560 },
  ],
  repeatMs: 2000,
};
const CHIME: RingPattern = {
  notes: [
    { at: 0, hz: 659.25 },
    { at: 0.19, hz: 880 },
    { at: 0.38, hz: 659.25 },
  ],
  repeatMs: 2600,
};
const SILENT: RingPattern = { notes: [], repeatMs: 0 };

export function ringPattern(ringtone: string): RingPattern {
  switch (ringtone) {
    case 'chime':
      return CHIME;
    case 'none':
      return SILENT;
    default:
      return ALVERCHAT;
  }
}
