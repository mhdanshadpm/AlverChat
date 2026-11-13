import { useId, type ReactNode } from 'react';
import { Icon, cx, type IconName } from './Icon';

export type EmptyStateProps = {
  /** Glyph inside the tinted art square. */
  icon?: IconName;
  /** Replaces the art square entirely. */
  art?: ReactNode;
  title: string;
  message?: ReactNode;
  /** Buttons under the copy, e.g. "New chat / New group / New call". */
  actions?: ReactNode;
  /** Tighter padding for in-panel empties. */
  compact?: boolean;
  className?: string;
};

/**
 * Empty / first-use state from the Themes & States board: art square, heading,
 * supporting line and a row of actions.
 */
export function EmptyState({
  icon,
  art,
  title,
  message,
  actions,
  compact = false,
  className,
}: EmptyStateProps) {
  const id = useId();
  return (
    <div
      aria-describedby={message ? `${id}-msg` : undefined}
      aria-labelledby={`${id}-title`}
      className={cx('ac-empty', compact && 'ac-empty--compact', className)}
      role="group"
    >
      {art ? (
        <span className="ac-empty__art">{art}</span>
      ) : icon ? (
        <span className="ac-empty__art">
          <Icon name={icon} size={compact ? 22 : 40} />
          {!compact ? <Icon className="ac-empty__spark" name="sparkles" size={20} /> : null}
        </span>
      ) : null}
      <h2 className="ac-empty__title" id={`${id}-title`}>
        {title}
      </h2>
      {message ? (
        <p className="ac-empty__message" id={`${id}-msg`}>
          {message}
        </p>
      ) : null}
      {actions ? <span className="ac-empty__actions">{actions}</span> : null}
    </div>
  );
}
