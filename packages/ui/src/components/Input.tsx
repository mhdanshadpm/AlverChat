import { forwardRef, useId, type InputHTMLAttributes, type ReactNode } from 'react';
import { Icon, cx, type IconName } from './Icon';

export type InputSize = 'sm' | 'md' | 'lg';

export type InputProps = Omit<InputHTMLAttributes<HTMLInputElement>, 'size'> & {
  /** Visible label. Omit it and pass `aria-label` for an icon-only field. */
  label?: string;
  hint?: ReactNode;
  /** Sets `aria-invalid` plus the red field treatment. */
  error?: string;
  /** Extra helper text under the field, wired with `aria-describedby`. */
  helper?: ReactNode;
  icon?: IconName;
  trailing?: ReactNode;
  /** Trailing glyph button, e.g. the eye that reveals a passphrase. */
  trailingIcon?: IconName;
  onTrailingClick?: () => void;
  trailingLabel?: string;
  size?: InputSize;
  /** Show the `n / maxLength` counter used on the profile form. */
  showCount?: boolean;
};

/** Text field with the Settings-board chrome: label, hint, error and counter. */
export const Input = forwardRef<HTMLInputElement, InputProps>(function Input(
  {
    label,
    hint,
    error,
    helper,
    icon,
    trailing,
    trailingIcon,
    onTrailingClick,
    trailingLabel,
    size = 'md',
    showCount = false,
    className,
    id,
    maxLength,
    value,
    defaultValue,
    disabled,
    ...rest
  },
  ref,
) {
  const autoId = useId();
  const inputId = id ?? autoId;
  const hintId = hint || helper ? `${inputId}-hint` : undefined;
  const describedBy = error ? `${inputId}-error` : hintId;
  const current =
    typeof value === 'string' ? value : typeof defaultValue === 'string' ? defaultValue : '';

  const field = (
    <span
      className={cx(
        'ac-input',
        size !== 'md' && `ac-input--${size}`,
        error && 'ac-input--invalid',
        disabled && 'ac-input--disabled',
        className,
      )}
    >
      {icon ? <Icon className="ac-input__icon" name={icon} size={16} /> : null}
      <input
        {...rest}
        ref={ref}
        aria-describedby={describedBy}
        aria-invalid={error ? true : undefined}
        disabled={disabled}
        id={inputId}
        maxLength={maxLength}
        defaultValue={defaultValue}
        value={value}
      />
      {trailingIcon ? (
        <button
          aria-label={trailingLabel}
          className="ac-search__clear"
          onClick={onTrailingClick}
          type="button"
        >
          <Icon name={trailingIcon} size={15} />
        </button>
      ) : null}
      {trailing ? <span className="ac-input__trailing">{trailing}</span> : null}
    </span>
  );

  if (!label && !hint && !helper && !error && !showCount) return field;

  return (
    <label className="ac-field">
      {label ? <span className="ac-field__label">{label}</span> : null}
      {field}
      {error ? (
        <span className="ac-field__error" id={`${inputId}-error`}>
          {error}
        </span>
      ) : hint || helper ? (
        <span className="ac-field__hint" id={hintId}>
          {hint ?? helper}
        </span>
      ) : null}
      {showCount && maxLength ? (
        <span className="ac-field__count">
          {current.length}/{maxLength}
        </span>
      ) : null}
    </label>
  );
});
