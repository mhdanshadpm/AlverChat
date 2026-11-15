import { useCallback, useEffect, useRef, useState, type KeyboardEvent } from 'react';
import { Icon, cx } from './Icon';

/* ------------------------------------------------------------------ *
 * HotkeyRecorder — the "Show / hide hotkey" field from
 * design/AlverChat_Settings_Design_System.png (Floating Icon & Bubbles).
 * Keyboard-only: listeners live on the field itself, never on `window`,
 * so a settings screen can never swallow the app's own shortcuts.
 * ------------------------------------------------------------------ */

/* ===== logic ===== */

/** Canonical modifier tokens, in display order. */
export const CHORD_MODIFIERS = ['CmdOrCtrl', 'Alt', 'Shift'] as const;

export type Chord = {
  modifiers: string[];
  key: string;
};

/** The subset of `KeyboardEvent` the recorder needs — keeps it testable in node. */
export type ChordInput = {
  key: string;
  ctrlKey: boolean;
  metaKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
};

const MODIFIER_ALIASES: Record<string, 'CmdOrCtrl' | 'Alt' | 'Shift'> = {
  cmd: 'CmdOrCtrl',
  cmdorctrl: 'CmdOrCtrl',
  commandorcontrol: 'CmdOrCtrl',
  command: 'CmdOrCtrl',
  meta: 'CmdOrCtrl',
  super: 'CmdOrCtrl',
  win: 'CmdOrCtrl',
  ctrl: 'CmdOrCtrl',
  control: 'CmdOrCtrl',
  alt: 'Alt',
  option: 'Alt',
  opt: 'Alt',
  shift: 'Shift',
};

/** Keys that can never be the trigger key of a shortcut. */
export const NON_TRIGGER_KEYS = [
  'Shift',
  'Control',
  'Alt',
  'Meta',
  'CapsLock',
  'Tab',
  'Enter',
  'Escape',
  'Backspace',
  'Delete',
  'ArrowUp',
  'ArrowDown',
  'ArrowLeft',
  'ArrowRight',
  'Unidentified',
];

/** True for a bare modifier press — the recorder keeps waiting for a chord. */
export function isModifierKey(key: string): boolean {
  return key === 'Shift' || key === 'Control' || key === 'Alt' || key === 'Meta';
}

/** `CmdOrCtrl+Alt+S` → `{ modifiers: ['CmdOrCtrl','Alt'], key: 'S' }`. */
export function parseChord(combo: string): Chord {
  const modifiers: string[] = [];
  let key = '';
  for (const raw of combo.split('+')) {
    const part = raw.trim();
    if (!part) continue;
    const alias = MODIFIER_ALIASES[part.toLowerCase()];
    if (alias) {
      if (!modifiers.includes(alias)) modifiers.push(alias);
      continue;
    }
    key = part === ' ' || part.toLowerCase() === 'space' ? 'Space' : part.toUpperCase();
  }
  return {
    modifiers: CHORD_MODIFIERS.filter((modifier) => modifiers.includes(modifier)),
    key,
  };
}

/** `{ modifiers: ['Alt','CmdOrCtrl'], key: 'S' }` → `CmdOrCtrl+Alt+S`. */
export function formatChord(chord: Chord): string {
  if (!chord.key) return '';
  const ordered = CHORD_MODIFIERS.filter((modifier) => chord.modifiers.includes(modifier));
  return [...ordered, chord.key].join('+');
}

/** Round-trips a stored combo through the canonical display form. */
export function displayChord(combo: string): string {
  return formatChord(parseChord(combo));
}

/** `F1`, `Space`, `s` → `S`; anything unrecordable → `null`. */
export function triggerKeyFor(key: string): string | null {
  if (NON_TRIGGER_KEYS.includes(key)) return null;
  if (key === ' ') return 'Space';
  return key.length === 1 ? key.toUpperCase() : key;
}

/** Chord from a key press, or `null` while only modifiers are down. */
export function chordFromEvent(event: ChordInput): Chord | null {
  const key = triggerKeyFor(event.key);
  if (!key) return null;
  const modifiers = [
    event.metaKey || event.ctrlKey ? 'CmdOrCtrl' : '',
    event.altKey ? 'Alt' : '',
    event.shiftKey ? 'Shift' : '',
  ].filter(Boolean);
  return { modifiers, key };
}

/** Resolve the cross-platform token into what the OS shortcut parser wants. */
export function platformChord(combo: string, isMac = false): string {
  const chord = parseChord(combo);
  if (!chord.key) return '';
  const modifiers = chord.modifiers.map((modifier) =>
    modifier === 'CmdOrCtrl' ? (isMac ? 'Command' : 'Ctrl') : modifier,
  );
  return [...modifiers, chord.key].join('+');
}

/** True when the combo carries at least one modifier plus a trigger key. */
export function isCompleteChord(combo: string): boolean {
  const chord = parseChord(combo);
  return chord.key.length > 0 && chord.modifiers.length > 0;
}

/* ===== component ===== */

export type HotkeyRecorderProps = {
  value: string;
  onChange: (next: string) => void;
  /** Accessible name, e.g. "Show / hide hotkey". */
  label: string;
  disabled?: boolean;
  placeholder?: string;
  recordingLabel?: string;
  clearLabel?: string;
  className?: string;
};

/**
 * Click (or focus + Enter) to arm the field, press a chord to set it, `Esc` to
 * cancel, `Backspace`/`Delete` to clear, blur to commit whatever is pending.
 */
export function HotkeyRecorder({
  value,
  onChange,
  label,
  disabled = false,
  placeholder = 'Press a shortcut',
  recordingLabel = 'Recording…',
  clearLabel = 'Clear shortcut',
  className,
}: HotkeyRecorderProps) {
  const [recording, setRecording] = useState(false);
  const [pending, setPending] = useState('');
  const fieldRef = useRef<HTMLButtonElement>(null);
  // `pending` is read from a blur handler, so keep the latest value reachable.
  const pendingRef = useRef(pending);
  pendingRef.current = pending;

  useEffect(() => {
    if (recording) fieldRef.current?.focus();
  }, [recording]);

  const commit = useCallback(
    (combo: string) => {
      setPending('');
      setRecording(false);
      onChange(combo);
    },
    [onChange],
  );

  const onKeyDown = useCallback(
    (event: KeyboardEvent<HTMLButtonElement>) => {
      if (disabled) return;

      if (!recording) {
        if (event.key === 'Enter' || event.key === ' ') {
          event.preventDefault();
          // Enter on a captured-but-uncommitted chord commits it without blurring.
          if (pending) commit(pending);
          else setRecording(true);
          return;
        }
        if (event.key === 'Backspace' || event.key === 'Delete') {
          event.preventDefault();
          commit('');
        }
        return;
      }

      event.preventDefault();
      if (event.key === 'Escape') {
        setPending('');
        setRecording(false);
        return;
      }
      if (event.key === 'Backspace' || event.key === 'Delete') {
        commit('');
        return;
      }
      if (isModifierKey(event.key)) return;

      const chord = chordFromEvent(event);
      if (!chord) return;
      const combo = formatChord(chord);
      if (!combo) return;
      // Held until blur (or Enter) so the recorder matches the board's behaviour.
      setPending(combo);
      setRecording(false);
    },
    [commit, disabled, pending, recording],
  );

  const shown = recording ? recordingLabel : displayChord(pending) || displayChord(value);
  const uncommitted = pending.length > 0;

  return (
    <span className={cx('ac-hotkey', recording && 'ac-hotkey--recording', uncommitted && 'ac-hotkey--pending', className)}>
      <button
        aria-disabled={disabled || undefined}
        aria-keyshortcuts={displayChord(pending) || value || undefined}
        className="ac-hotkey__field"
        disabled={disabled}
        onBlur={() => {
          setRecording(false);
          // Blur commits the captured chord; Esc is the only way to discard it.
          if (pendingRef.current) commit(pendingRef.current);
        }}
        onClick={() => setRecording((current) => !current)}
        onKeyDown={onKeyDown}
        ref={fieldRef}
        type="button"
      >
        <span className="ac-sr-only">{label}</span>
        <span className={cx('ac-hotkey__value', !shown && 'ac-hotkey__value--empty')}>
          {shown || placeholder}
        </span>
        <span aria-live="polite" className="ac-hotkey__state">
          {recording ? recordingLabel : ''}
        </span>
      </button>
      {(value || uncommitted) && !recording ? (
        <button
          aria-label={clearLabel}
          className="ac-hotkey__clear"
          disabled={disabled}
          onClick={() => {
            fieldRef.current?.focus();
            commit('');
          }}
          type="button"
        >
          <Icon name="x" size={13} />
        </button>
      ) : null}
    </span>
  );
}
