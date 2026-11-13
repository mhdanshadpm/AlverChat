import { forwardRef, type InputHTMLAttributes, type ReactNode } from 'react';
import { Icon, cx } from './Icon';

export type SearchInputProps = Omit<
  InputHTMLAttributes<HTMLInputElement>,
  'type' | 'size' | 'onChange'
> & {
  value: string;
  onChange: (next: string) => void;
  /** Keyboard hint rendered as a `<kbd>` chip, e.g. "⌘ K". */
  shortcut?: string;
  onClear?: () => void;
  /** Rendered inside the field on the trailing side. */
  trailing?: ReactNode;
  clearLabel?: string;
};

/**
 * The list-pane search field: magnifier, input, optional clear button and
 * optional shortcut chip. `shortcut` is announced with the field.
 */
export const SearchInput = forwardRef<HTMLInputElement, SearchInputProps>(function SearchInput(
  { value, onChange, shortcut, onClear, trailing, clearLabel = 'Clear search', className, placeholder = 'Search…', ...rest },
  ref,
) {
  return (
    <span className={cx('ac-input', 'ac-search', className)} role="search">
      <Icon className="ac-input__icon" name="search" size={16} />
      <input
        {...rest}
        ref={ref}
        aria-label={rest['aria-label'] ?? placeholder}
        onChange={(event) => onChange(event.target.value)}
        placeholder={placeholder}
        type="search"
        value={value}
      />
      {trailing ? <span className="ac-input__trailing">{trailing}</span> : null}
      {value.length > 0 && onClear ? (
        <button
          aria-label={clearLabel}
          className="ac-search__clear"
          onClick={onClear}
          type="button"
        >
          <Icon name="x" size={14} />
        </button>
      ) : null}
      {shortcut ? <kbd className="ac-search__kbd">{shortcut}</kbd> : null}
    </span>
  );
});
