import { useCallback, useId, useRef, type KeyboardEvent, type ReactNode } from 'react';
import { Icon, cx, type IconName } from './Icon';
import { formatCount, nextIndexOf, typeaheadIndex } from './logic';

export type TabItem = {
  id: string;
  label: string;
  icon?: IconName;
  /** Unread count chip. */
  count?: number;
  disabled?: boolean;
  /** Panel body; omit for a header-only tab strip. */
  content?: ReactNode;
};

export type TabsProps = {
  tabs: ReadonlyArray<TabItem>;
  value: string;
  onChange: (next: string) => void;
  label?: string;
  /** Rounded pill tabs instead of the underline strip. */
  variant?: 'underline' | 'pill';
  className?: string;
};

/**
 * Underline tab strip. Follows the WAI-ARIA tabs pattern: roving tabindex,
 * arrow keys, Home/End and typeahead, with `aria-selected` on each tab and
 * `aria-controls` linking to the panel.
 */
export function Tabs({ tabs, value, onChange, label, variant = 'underline', className }: TabsProps) {
  const baseId = useId();
  const listRef = useRef<HTMLDivElement>(null);
  const enabled = tabs.filter((tab) => !tab.disabled);

  const focusAt = useCallback(
    (index: number) => {
      listRef.current
        ?.querySelector<HTMLButtonElement>(`[data-ac-tab-index="${index}"]`)
        ?.focus();
    },
    [],
  );

  const onKeyDown = useCallback(
    (event: KeyboardEvent<HTMLDivElement>) => {
      if (enabled.length === 0) return;
      const current = enabled.findIndex((tab) => tab.id === value);
      let delta = 0;
      if (event.key === 'ArrowRight' || event.key === 'ArrowDown') delta = 1;
      else if (event.key === 'ArrowLeft' || event.key === 'ArrowUp') delta = -1;
      else if (event.key === 'Home') delta = -enabled.length;
      else if (event.key === 'End') delta = enabled.length - 1;
      else if (event.key.length === 1) {
        const hit = typeaheadIndex(
          enabled.map((tab) => tab.label),
          event.key,
          current < 0 ? 0 : current,
        );
        if (hit < 0) return;
        event.preventDefault();
        onChange(enabled[hit].id);
        focusAt(tabs.findIndex((tab) => tab.id === enabled[hit].id));
        return;
      } else return;
      event.preventDefault();
      const offset = nextIndexOf(current < 0 ? 0 : current, delta, enabled.length);
      const target = enabled[offset];
      if (!target) return;
      onChange(target.id);
      focusAt(tabs.findIndex((tab) => tab.id === target.id));
    },
    [enabled, focusAt, onChange, tabs, value],
  );

  const active = tabs.find((tab) => tab.id === value);

  return (
    <div className={cx('ac-tabs', variant === 'pill' && 'ac-tabs--pill', className)}>
      <div
        aria-label={label}
        aria-orientation="horizontal"
        className="ac-tabs__list"
        onKeyDown={onKeyDown}
        ref={listRef}
        role="tablist"
      >
        {tabs.map((tab, index) => {
          const selected = tab.id === value;
          const panelId = `${baseId}-panel-${tab.id}`;
          return (
            <button
              aria-controls={tab.content ? panelId : undefined}
              aria-disabled={tab.disabled || undefined}
              aria-selected={selected}
              className="ac-tab"
              data-ac-tab-index={index}
              disabled={tab.disabled}
              id={`${baseId}-tab-${tab.id}`}
              key={tab.id}
              onClick={() => onChange(tab.id)}
              role="tab"
              tabIndex={selected ? 0 : -1}
              type="button"
            >
              {tab.icon ? <Icon name={tab.icon} size={16} /> : null}
              {tab.label}
              {typeof tab.count === 'number' && tab.count > 0 ? (
                <span className="ac-badge ac-badge--accent">{formatCount(tab.count)}</span>
              ) : null}
            </button>
          );
        })}
      </div>
      {active?.content ? (
        <div
          aria-labelledby={`${baseId}-tab-${active.id}`}
          className="ac-tab__panel"
          id={`${baseId}-panel-${active.id}`}
          role="tabpanel"
          tabIndex={0}
        >
          {active.content}
        </div>
      ) : null}
    </div>
  );
}
