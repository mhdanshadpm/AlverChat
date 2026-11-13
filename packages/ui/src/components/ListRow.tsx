import type { HTMLAttributes, ReactNode } from 'react';
import { Icon, cx, type IconName } from './Icon';

export type ListRowProps = Omit<HTMLAttributes<HTMLElement>, 'title'> & {
  /** Leading glyph. The board shows a tinted square for settings rows. */
  icon?: IconName;
  /** Replaces the glyph with any node, e.g. an Avatar. */
  leading?: ReactNode;
  /** Tint the leading glyph square (default) or keep it flat. */
  leadingTone?: 'soft' | 'plain';
  title: ReactNode;
  hint?: ReactNode;
  /** Trailing control (Toggle, Select, Slider…) or text such as a timestamp. */
  trailing?: ReactNode;
  /** Adds the trailing chevron and makes the whole row a button. */
  onClick?: () => void;
  chevron?: boolean;
  active?: boolean;
  disabled?: boolean;
  /** `aria-current` for the selected sidebar row. */
  current?: boolean;
};

/**
 * The Settings-board row: leading icon, title + hint stack, trailing control.
 * Renders a `<button>` when it has an `onClick`, otherwise a plain row.
 */
export function ListRow({
  icon,
  leading,
  leadingTone = 'soft',
  title,
  hint,
  trailing,
  onClick,
  chevron = false,
  active = false,
  disabled = false,
  current = false,
  className,
  ...rest
}: ListRowProps) {
  const body = (
    <>
      {leading ? (
        <span className="ac-row__leading">{leading}</span>
      ) : icon ? (
        <span
          className={cx(
            'ac-row__icon',
            leadingTone === 'plain' && 'ac-row__icon--plain',
          )}
        >
          <Icon name={icon} size={18} />
        </span>
      ) : null}
      <span className="ac-row__body">
        <span className="ac-row__title">{title}</span>
        {hint ? <span className="ac-row__hint">{hint}</span> : null}
      </span>
      {trailing ? <span className="ac-row__trailing">{trailing}</span> : null}
      {chevron ? <Icon className="ac-row__chevron" name="chevronRight" size={16} /> : null}
    </>
  );

  const classes = cx(
    'ac-row',
    onClick && 'ac-row--button',
    active && 'ac-row--active',
    disabled && 'ac-row--disabled',
    className,
  );

  if (onClick) {
    return (
      <button
        {...rest}
        aria-current={current ? 'true' : undefined}
        aria-disabled={disabled || undefined}
        className={classes}
        disabled={disabled}
        onClick={onClick}
        type="button"
      >
        {body}
      </button>
    );
  }

  return (
    <div
      {...rest}
      aria-current={current ? 'true' : undefined}
      className={classes}
    >
      {body}
    </div>
  );
}

export type ListGroupProps = {
  children: ReactNode;
  /** Section caption above the rows, e.g. "PINNED (2)". */
  section?: string;
  className?: string;
};

/** Bordered card that groups ListRows, as used across the Settings screens. */
export function ListGroup({ children, section, className }: ListGroupProps) {
  return (
    <div className={cx('ac-row__group', className)}>
      {section ? <div className="ac-row__section">{section}</div> : null}
      {children}
    </div>
  );
}
