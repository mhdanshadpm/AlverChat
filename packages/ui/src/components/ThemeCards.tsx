import { useCallback, useId, useRef, type KeyboardEvent } from 'react';
import { Icon, cx } from './Icon';
import { nextIndexOf } from './logic';

/* ------------------------------------------------------------------ *
 * ThemeCards — the Light / Dark / System *card* radios from
 * design/AlverChat_Settings_Design_System.png (General section).
 * The library already ships `Segmented` for the compact variant; the board
 * pairs it with these preview-card radios, which had no representation.
 * ------------------------------------------------------------------ */

/* ===== logic ===== */

export type ThemeChoice = 'light' | 'dark' | 'system';

export type ThemeOption = {
  value: ThemeChoice;
  label: string;
};

/** The three cards, in board order. */
export const THEME_OPTIONS: readonly ThemeOption[] = [
  { value: 'light', label: 'Light' },
  { value: 'dark', label: 'Dark' },
  { value: 'system', label: 'System' },
];

/** Options with the caller's labels applied, keeping the board order. */
export function themedOptions(labels: Partial<Record<ThemeChoice, string>> = {}): ThemeOption[] {
  return THEME_OPTIONS.map((option) => ({
    ...option,
    label: labels[option.value] ?? option.label,
  }));
}

/** Narrow any stored string onto the three real theme values. */
export function themeChoiceFor(value: string | null | undefined): ThemeChoice {
  return value === 'light' || value === 'dark' ? value : 'system';
}

/** Index that receives `tabindex=0`: the selection, else the first card. */
export function themeTabIndex(value: ThemeChoice, options: readonly ThemeOption[]): number {
  const index = options.findIndex((option) => option.value === value);
  return index < 0 ? 0 : index;
}

/* ===== swatch ===== */

/**
 * Mini window preview built entirely from tokens, including the diagonal split
 * used by the System card. No colour is hard-coded here.
 */
function ThemeSwatch({ variant }: { variant: ThemeChoice }) {
  const clipId = `${useId()}-clip`;
  const bars = (bg: string, bubble: string, soft: string) => (
    <>
      <rect fill={bg} height="72" rx="6" width="120" />
      <rect fill={soft} height="10" rx="3" width="34" x="10" y="9" />
      <rect fill={bubble} height="13" rx="4" width="58" x="44" y="27" />
      <rect fill={soft} height="13" rx="4" width="46" x="10" y="47" />
    </>
  );

  return (
    <svg
      aria-hidden
      className="ac-themecard__svg"
      preserveAspectRatio="xMidYMid slice"
      viewBox="0 0 120 72"
      xmlns="http://www.w3.org/2000/svg"
    >
      {variant === 'light'
        ? bars('var(--ac-theme-light-bg)', 'var(--ac-theme-accent)', 'var(--ac-theme-light-fill)')
        : null}
      {variant === 'dark'
        ? bars('var(--ac-theme-dark-bg)', 'var(--ac-theme-accent)', 'var(--ac-theme-dark-fill)')
        : null}
      {variant === 'system' ? (
        <>
          <clipPath id={clipId}>
            <polygon points="74,0 120,0 120,72 30,72" />
          </clipPath>
          {bars('var(--ac-theme-light-bg)', 'var(--ac-theme-accent)', 'var(--ac-theme-light-fill)')}
          <g clipPath={`url(#${clipId})`}>
            {bars('var(--ac-theme-dark-bg)', 'var(--ac-theme-accent)', 'var(--ac-theme-dark-fill)')}
          </g>
        </>
      ) : null}
    </svg>
  );
}

/* ===== component ===== */

export type ThemeCardsProps = {
  value: ThemeChoice;
  onChange: (next: ThemeChoice) => void;
  disabled?: boolean;
  /** Accessible name for the radio group. */
  label?: string;
  /** Localised card captions; defaults to the English board labels. */
  labels?: Partial<Record<ThemeChoice, string>>;
  className?: string;
};

/**
 * Real radio-group semantics: `role="radiogroup"` + `role="radio"` +
 * `aria-checked`, roving tabindex and arrow/Home/End navigation, so the cards
 * behave exactly like the native single-choice widget they replace.
 */
export function ThemeCards({
  value,
  onChange,
  disabled = false,
  label = 'Theme',
  labels,
  className,
}: ThemeCardsProps) {
  const options = themedOptions(labels);
  const tabbable = themeTabIndex(value, options);
  const groupRef = useRef<HTMLDivElement>(null);

  const select = useCallback(
    (next: ThemeChoice) => {
      if (!disabled) onChange(next);
    },
    [disabled, onChange],
  );

  const focusAt = useCallback((index: number) => {
    groupRef.current
      ?.querySelector<HTMLButtonElement>(`[data-ac-theme-index="${index}"]`)
      ?.focus();
  }, []);

  const onKeyDown = useCallback(
    (event: KeyboardEvent<HTMLDivElement>) => {
      if (disabled) return;
      let delta = 0;
      if (event.key === 'ArrowRight' || event.key === 'ArrowDown') delta = 1;
      else if (event.key === 'ArrowLeft' || event.key === 'ArrowUp') delta = -1;
      else if (event.key === 'Home') delta = -options.length;
      else if (event.key === 'End') delta = options.length - 1;
      else return;
      event.preventDefault();
      const offset = nextIndexOf(options.findIndex((option) => option.value === value), delta, options.length);
      const target = options[offset];
      if (!target) return;
      select(target.value);
      focusAt(offset);
    },
    [disabled, focusAt, options, select, value],
  );

  return (
    <div
      aria-disabled={disabled || undefined}
      aria-label={label}
      className={cx('ac-themecards', className)}
      onKeyDown={onKeyDown}
      ref={groupRef}
      role="radiogroup"
    >
      {options.map((option, index) => {
        const checked = option.value === value;
        return (
          <button
            aria-checked={checked}
            aria-disabled={disabled || undefined}
            className={cx('ac-themecard', checked && 'ac-themecard--on')}
            data-ac-theme-index={index}
            disabled={disabled}
            key={option.value}
            onClick={() => select(option.value)}
            role="radio"
            tabIndex={index === tabbable ? 0 : -1}
            type="button"
          >
            <span className="ac-themecard__swatch">
              <ThemeSwatch variant={option.value} />
            </span>
            <span className="ac-themecard__label">{option.label}</span>
            {checked ? (
              <span aria-hidden className="ac-themecard__check">
                <Icon name="check" size={11} strokeWidth={3} />
              </span>
            ) : null}
          </button>
        );
      })}
    </div>
  );
}
