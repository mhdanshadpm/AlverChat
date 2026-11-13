import { useCallback, useId, useRef, type KeyboardEvent } from 'react';
import { Icon, cx, type IconName } from './Icon';
import { nextIndexOf } from './logic';

export type SegmentedOption = {
  value: string;
  label: string;
  icon?: IconName;
  disabled?: boolean;
};

export type SegmentedProps = {
  options: ReadonlyArray<SegmentedOption | string>;
  value: string;
  onChange: (next: string) => void;
  /** Accessible name for the group, e.g. "Theme". */
  label?: string;
  block?: boolean;
  disabled?: boolean;
  className?: string;
};

/**
 * Light / Dark / System segmented control from the Settings board.
 * Exposed as a radiogroup so arrow keys move the selection, matching the
 * native single-choice semantics of the widget.
 */
export function Segmented({
  options,
  value,
  onChange,
  label,
  block = false,
  disabled = false,
  className,
}: SegmentedProps) {
  const groupId = useId();
  const listRef = useRef<HTMLDivElement>(null);

  const items = options.map((option) =>
    typeof option === 'string' ? { value: option, label: option } : option,
  );
  const enabledIndexes = items
    .map((item, index) => ({ item, index }))
    .filter((entry) => !entry.item.disabled && !disabled)
    .map((entry) => entry.index);

  const select = useCallback(
    (next: string) => {
      if (!disabled) onChange(next);
    },
    [disabled, onChange],
  );

  const focusAt = useCallback((index: number) => {
    const node = listRef.current?.querySelector<HTMLButtonElement>(
      `[data-ac-segmented-index="${index}"]`,
    );
    node?.focus();
  }, []);

  const onKeyDown = useCallback(
    (event: KeyboardEvent<HTMLDivElement>) => {
      if (enabledIndexes.length === 0) return;
      const current = enabledIndexes.indexOf(items.findIndex((item) => item.value === value));
      let delta = 0;
      if (event.key === 'ArrowRight' || event.key === 'ArrowDown') delta = 1;
      else if (event.key === 'ArrowLeft' || event.key === 'ArrowUp') delta = -1;
      else if (event.key === 'Home') delta = -enabledIndexes.length;
      else if (event.key === 'End') delta = enabledIndexes.length - 1;
      else return;
      event.preventDefault();
      const offset = nextIndexOf(current < 0 ? 0 : current, delta, enabledIndexes.length);
      const target = enabledIndexes[offset];
      if (target === undefined) return;
      select(items[target].value);
      focusAt(target);
    },
    [enabledIndexes, focusAt, items, select, value],
  );

  return (
    <div
      aria-disabled={disabled || undefined}
      aria-label={label}
      className={cx('ac-segmented', block && 'ac-segmented--block', className)}
      id={groupId}
      onKeyDown={onKeyDown}
      ref={listRef}
      role="radiogroup"
    >
      {items.map((item, index) => {
        const selected = item.value === value;
        const off = disabled || item.disabled === true;
        return (
          <button
            aria-checked={selected}
            aria-disabled={off || undefined}
            className={cx('ac-segmented__option', selected && 'ac-segmented__option--on')}
            data-ac-segmented-index={index}
            disabled={off}
            key={item.value}
            onClick={() => select(item.value)}
            role="radio"
            tabIndex={selected ? 0 : -1}
            type="button"
          >
            {item.icon ? <Icon name={item.icon} size={14} /> : null}
            {item.label}
          </button>
        );
      })}
    </div>
  );
}
