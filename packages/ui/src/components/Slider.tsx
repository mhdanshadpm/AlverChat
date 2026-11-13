import { useId, type CSSProperties } from 'react';
import { cx } from './Icon';
import { clampPercent } from './logic';

export type SliderProps = {
  value: number;
  min?: number;
  max?: number;
  step?: number;
  onChange: (next: number) => void;
  /** Accessible name, e.g. "Bubble auto-hide delay". */
  label: string;
  /** Formats the read-out and the floating value bubble. */
  format?: (value: number) => string;
  /** Keeps the value bubble visible on hover/focus. */
  bubble?: boolean;
  /** Show the numeric read-out to the right of the rail. */
  showValue?: boolean;
  disabled?: boolean;
  className?: string;
  id?: string;
};

/**
 * Range control from the Settings board: accent-filled rail, white thumb with a
 * 2px accent border, and a value bubble on hover/focus.
 */
export function Slider({
  value,
  min = 0,
  max = 100,
  step = 1,
  onChange,
  label,
  format,
  bubble = true,
  showValue = true,
  disabled = false,
  className,
  id,
}: SliderProps) {
  const autoId = useId();
  const sliderId = id ?? autoId;
  const span = max - min;
  const percent = clampPercent(span === 0 ? 0 : ((value - min) / span) * 100);
  const text = format ? format(value) : `${percent}%`;
  const railStyle = { '--ac-slider-fill': `${percent}%` } as CSSProperties;

  return (
    <span className={cx('ac-slider', className)}>
      <span className="ac-slider__rail" style={railStyle}>
        <input
          aria-label={label}
          aria-valuetext={text}
          className="ac-slider__input"
          disabled={disabled}
          id={sliderId}
          max={max}
          min={min}
          onChange={(event) => onChange(Number(event.target.value))}
          step={step}
          type="range"
          value={value}
        />
      </span>
      {showValue ? (
        <span className="ac-slider__value">
          {bubble ? <span aria-hidden className="ac-slider__bubble">
            {text}
          </span> : null}
          {text}
        </span>
      ) : null}
    </span>
  );
}
