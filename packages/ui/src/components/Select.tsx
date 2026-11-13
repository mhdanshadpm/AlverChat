import { forwardRef, useId, type ReactNode, type SelectHTMLAttributes } from 'react';
import { Icon, cx } from './Icon';

export type SelectOption = {
  value: string;
  label: string;
  disabled?: boolean;
};

export type SelectProps = Omit<SelectHTMLAttributes<HTMLSelectElement>, 'children' | 'size'> & {
  /** Visible label rendered above the control. */
  label?: string;
  hint?: ReactNode;
  /** Plain strings or `{ value, label }` objects. */
  options: ReadonlyArray<string | SelectOption>;
  placeholder?: string;
  block?: boolean;
};

/** Normalises the two accepted option shapes into one. */
export function toSelectOptions(
  options: ReadonlyArray<string | SelectOption>,
): SelectOption[] {
  return options.map((option) =>
    typeof option === 'string' ? { value: option, label: option } : option,
  );
}

/**
 * Native dropdown styled like the Settings board "Dropdown (Select)":
 * surface-2 field, 8px radius, trailing chevron. Native keeps full keyboard
 * and screen-reader support for free.
 */
export const Select = forwardRef<HTMLSelectElement, SelectProps>(function Select(
  { label, hint, options, placeholder, block = false, className, id, disabled, ...rest },
  ref,
) {
  const autoId = useId();
  const selectId = id ?? autoId;
  const items = toSelectOptions(options);

  const control = (
    <span className={cx('ac-select', block && 'ac-select--block', className)}>
      <select
        {...rest}
        ref={ref}
        aria-describedby={hint ? `${selectId}-hint` : undefined}
        aria-label={label ? undefined : rest['aria-label']}
        disabled={disabled}
        id={selectId}
      >
        {placeholder ? (
          <option disabled value="">
            {placeholder}
          </option>
        ) : null}
        {items.map((option) => (
          <option disabled={option.disabled} key={option.value} value={option.value}>
            {option.label}
          </option>
        ))}
      </select>
      <Icon className="ac-select__chevron" name="chevronDown" size={16} />
    </span>
  );

  if (!label && !hint) return control;

  return (
    <label className="ac-field">
      {label ? <span className="ac-field__label">{label}</span> : null}
      {control}
      {hint ? (
        <span className="ac-field__hint" id={`${selectId}-hint`}>
          {hint}
        </span>
      ) : null}
    </label>
  );
});
