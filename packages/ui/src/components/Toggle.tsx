import { forwardRef, useCallback, useState, type KeyboardEvent } from 'react';
import { cx } from './Icon';

export type ToggleProps = {
  checked: boolean;
  onChange: (next: boolean) => void;
  /** Accessible name for the switch. */
  label: string;
  disabled?: boolean;
  /** Optional id so a `<ListRow>` label can point at the control. */
  id?: string;
  name?: string;
  className?: string;
};

/**
 * The Settings-board switch: 37x20 pill, 16px thumb, accent when on.
 * Implemented as `role="switch"` so it announces on/off rather than checked.
 */
export const Toggle = forwardRef<HTMLButtonElement, ToggleProps>(function Toggle(
  { checked, onChange, label, disabled = false, id, name, className },
  ref,
) {
  const toggle = useCallback(() => {
    if (!disabled) onChange(!checked);
  }, [checked, disabled, onChange]);

  const onKeyDown = useCallback(
    (event: KeyboardEvent<HTMLButtonElement>) => {
      if (event.key === 'Enter' || event.key === ' ') {
        event.preventDefault();
        toggle();
      }
    },
    [toggle],
  );

  return (
    <button
      aria-checked={checked}
      aria-disabled={disabled || undefined}
      aria-label={label}
      className={cx('ac-toggle', checked && 'ac-toggle--on', className)}
      disabled={disabled}
      id={id}
      name={name}
      onClick={toggle}
      onKeyDown={onKeyDown}
      ref={ref}
      role="switch"
      type="button"
    >
      <span className="ac-toggle__thumb" />
    </button>
  );
});

export type UncontrolledToggleProps = {
  defaultChecked?: boolean;
  onChange?: (next: boolean) => void;
  label: string;
  disabled?: boolean;
  id?: string;
  className?: string;
};

/** Convenience wrapper for boards that only need a local on/off. */
export function UncontrolledToggle({
  defaultChecked = false,
  onChange,
  ...rest
}: UncontrolledToggleProps) {
  const [checked, setChecked] = useState(defaultChecked);
  return (
    <Toggle
      {...rest}
      checked={checked}
      onChange={(next) => {
        setChecked(next);
        onChange?.(next);
      }}
    />
  );
}
