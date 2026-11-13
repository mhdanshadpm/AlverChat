import type { ReactNode } from 'react';
import { cx } from './Icon';
import { clampPercent } from './logic';

export type ProgressTone = 'accent' | 'success' | 'danger' | 'warning';

export type ProgressBarProps = {
  /** 0..100. Omit (or pass null) for the indeterminate shimmer. */
  value?: number | null;
  tone?: ProgressTone;
  /** Label above the rail, e.g. the file name. */
  label?: ReactNode;
  /** Right-aligned read-out; defaults to `n%`. */
  valueText?: ReactNode;
  /** Accessible name; falls back to a textified `label`. */
  ariaLabel?: string;
  className?: string;
};

/**
 * 4px rail with the accent gradient fill, used for file transfers and the
 * download progress line inside a file bubble.
 */
export function ProgressBar({
  value,
  tone = 'accent',
  label,
  valueText,
  ariaLabel,
  className,
}: ProgressBarProps) {
  const indeterminate = typeof value !== 'number' || !Number.isFinite(value);
  const percent = indeterminate ? 0 : clampPercent(value);
  const readout = valueText ?? `${percent}%`;
  return (
    <span
      className={cx(
        'ac-progress',
        tone !== 'accent' && `ac-progress--${tone}`,
        indeterminate && 'ac-progress--indeterminate',
        className,
      )}
    >
      {label || !indeterminate ? (
        <span className="ac-progress__head">
          <span>{label}</span>
          <span className="ac-progress__value">{readout}</span>
        </span>
      ) : null}
      <span
        aria-label={ariaLabel ?? (typeof label === 'string' ? label : 'Progress')}
        aria-valuemax={100}
        aria-valuemin={0}
        aria-valuenow={indeterminate ? undefined : percent}
        className="ac-progress__track"
        role="progressbar"
      >
        <span className="ac-progress__bar" style={{ width: `${percent}%` }} />
      </span>
    </span>
  );
}
