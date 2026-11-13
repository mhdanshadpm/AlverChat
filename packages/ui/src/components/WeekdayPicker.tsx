import { useCallback, type KeyboardEvent } from 'react';
import { cx } from './Icon';

/* ------------------------------------------------------------------ *
 * WeekdayPicker — the DND weekday chips from
 * design/AlverChat_Settings_Design_System.png (Notifications section).
 * The strings are the ones the Rust side compares against
 * (`Local::now().format("%a")` in apps/desktop/src-tauri/src/platform.rs),
 * so `value` must stay in `Mon`…`Sun` form.
 * ------------------------------------------------------------------ */

/* ===== logic ===== */

/** Monday-first week, matching `%a` output. */
export const WEEKDAY_DAYS: readonly string[] = [
  'Mon',
  'Tue',
  'Wed',
  'Thu',
  'Fri',
  'Sat',
  'Sun',
];

/** Weekends, so a screen can offer "weekdays only" / "every day" presets. */
export function weekendDays(all: readonly string[] = WEEKDAY_DAYS): string[] {
  return all.filter((day) => day === 'Sat' || day === 'Sun');
}

/** Weekday order of `all`, with unknown values kept last in insertion order. */
export function orderDays(days: readonly string[], all: readonly string[] = WEEKDAY_DAYS): string[] {
  const rank = (day: string): number => {
    const index = all.indexOf(day);
    return index < 0 ? all.length : index;
  };
  return [...days].sort((left, right) => rank(left) - rank(right));
}

/** Add or remove one day, always returning a canonically ordered copy. */
export function toggleDay(
  day: string,
  selected: readonly string[],
  all: readonly string[] = WEEKDAY_DAYS,
): string[] {
  const next = selected.includes(day)
    ? selected.filter((item) => item !== day)
    : [...selected, day];
  return orderDays(next, all);
}

/** True when every offered day is on. */
export function isEveryDay(selected: readonly string[], all: readonly string[] = WEEKDAY_DAYS): boolean {
  return all.length > 0 && all.every((day) => selected.includes(day));
}

/** True when the selection is exactly the weekdays (the board default). */
export function isWeekdaysOnly(
  selected: readonly string[],
  all: readonly string[] = WEEKDAY_DAYS,
): boolean {
  const weekend = weekendDays(all);
  return (
    all.every((day) => (weekend.includes(day) ? !selected.includes(day) : selected.includes(day))) &&
    selected.every((day) => all.includes(day))
  );
}

/* ===== component ===== */

export type WeekdayPickerProps = {
  /** Selected day abbreviations, e.g. `['Mon','Tue','Wed','Thu','Fri']`. */
  value: string[];
  onChange: (next: string[]) => void;
  /** Offered days; defaults to the Monday-first week. */
  days?: readonly string[];
  /** Display captions per day; defaults to the stored abbreviation. */
  labels?: Record<string, string>;
  /** Accessible name for the chip group. */
  legend?: string;
  disabled?: boolean;
  className?: string;
};

/**
 * Multi-select chip row. Each chip is a toggle button with `aria-pressed`, and
 * the group is a `role="group"` so screen readers announce the caption once.
 */
export function WeekdayPicker({
  value,
  onChange,
  days = WEEKDAY_DAYS,
  labels = {},
  legend = 'Days',
  disabled = false,
  className,
}: WeekdayPickerProps) {
  const toggle = useCallback(
    (day: string) => {
      if (!disabled) onChange(toggleDay(day, value, days));
    },
    [disabled, days, onChange, value],
  );

  const onKeyDown = useCallback(
    (event: KeyboardEvent<HTMLButtonElement>, day: string) => {
      if (event.key === ' ' || event.key === 'Enter') {
        // Buttons already fire onClick, this only keeps Space from scrolling.
        event.preventDefault();
        toggle(day);
      }
    },
    [toggle],
  );

  return (
    <div aria-disabled={disabled || undefined} aria-label={legend} className={cx('ac-weekdays', className)} role="group">
      {days.map((day) => {
        const pressed = value.includes(day);
        return (
          <button
            aria-pressed={pressed}
            aria-disabled={disabled || undefined}
            className={cx('ac-weekdays__chip', pressed && 'ac-weekdays__chip--on')}
            disabled={disabled}
            key={day}
            onClick={() => toggle(day)}
            onKeyDown={(event) => onKeyDown(event, day)}
            type="button"
          >
            {labels[day] ?? day}
          </button>
        );
      })}
    </div>
  );
}
