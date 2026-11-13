import type { ReactNode } from 'react';
import { cx } from './Icon';
import { formatCount } from './logic';

export type BadgeTone = 'neutral' | 'accent' | 'danger' | 'success' | 'warning' | 'outline';

export type BadgeProps = {
  children?: ReactNode;
  tone?: BadgeTone;
  /** Numeric count; renders `max+` past the cap. Use `children` for text badges. */
  count?: number;
  max?: number;
  /** Small dot with no label, e.g. the presence marker on the floating icon. */
  dot?: boolean;
  /** Unread attention pulse (1.0 -> 1.08 -> 1.0 over 1.5s) from the icon-states board. */
  pulse?: boolean;
  /** Accessible label for icon-only or dot badges. */
  label?: string;
  className?: string;
};

/** Count pill / status chip. */
export function Badge({
  children,
  tone = 'neutral',
  count,
  max = 99,
  dot = false,
  pulse = false,
  label,
  className,
}: BadgeProps) {
  if (dot) {
    return (
      <span
        aria-hidden={label ? undefined : true}
        aria-label={label}
        className={cx('ac-badge', 'ac-badge--dot', tone !== 'neutral' && `ac-badge--${tone}`, className)}
      />
    );
  }
  const text = typeof count === 'number' ? formatCount(count, max) : children;
  return (
    <span aria-label={label} className={cx('ac-badge', tone !== 'neutral' && `ac-badge--${tone}`, pulse && 'ac-badge--pulse', className)} role="status">
      {text}
    </span>
  );
}

export type BadgeCountProps = {
  children: ReactNode;
  /** Badge pinned to the top-right of this element. */
  count?: number;
  max?: number;
  tone?: BadgeTone;
  pulse?: boolean;
  label?: string;
  hidden?: boolean;
  className?: string;
};

/** Wraps a control with an absolutely positioned count badge (rail icons, dock icon). */
export function BadgeCount({
  children,
  count,
  max = 99,
  tone = 'danger',
  pulse = false,
  label,
  hidden = false,
  className,
}: BadgeCountProps) {
  const show = !hidden && typeof count === 'number' && count > 0;
  return (
    <span className={cx('ac-badge__anchor', className)}>
      {children}
      {show ? (
        <Badge count={count} label={label} max={max} pulse={pulse} tone={tone} />
      ) : null}
    </span>
  );
}
