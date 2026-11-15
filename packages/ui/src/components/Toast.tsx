import { useEffect, type ReactNode } from 'react';
import { ns, registerNamespace } from '../i18n';
import systemCatalog from '../i18n/en/system';
import { Icon, cx, type IconName } from './Icon';

registerNamespace('system', systemCatalog);

const text = ns('system');

export type ToastTone = 'neutral' | 'accent' | 'success' | 'danger';

export type ToastProps = {
  /** Body text. */
  message: ReactNode;
  title?: ReactNode;
  tone?: ToastTone;
  icon?: IconName;
  /** Single action, e.g. "Retry" or "Undo". */
  action?: { label: string; onClick: () => void };
  onDismiss?: () => void;
  /** Auto-dismiss after n ms; 0 keeps the toast until dismissed. */
  durationMs?: number;
  className?: string;
};

const TONE_ICON: Record<ToastTone, IconName> = {
  neutral: 'checkCheck',
  accent: 'info',
  success: 'checkCheck',
  danger: 'alertCircle',
};

/**
 * Dark pill toast from the showcase board. `role="status"` (or `alert` for
 * danger) so it is announced without stealing focus.
 */
export function Toast({
  message,
  title,
  tone = 'neutral',
  icon,
  action,
  onDismiss,
  durationMs = 3200,
  className,
}: ToastProps) {
  useEffect(() => {
    if (durationMs <= 0 || !onDismiss) return;
    const handle = setTimeout(onDismiss, durationMs);
    return () => clearTimeout(handle);
  }, [durationMs, onDismiss]);

  return (
    <div
      className={cx('ac-toast', tone !== 'neutral' && `ac-toast--${tone}`, className)}
      role={tone === 'danger' ? 'alert' : 'status'}
    >
      <Icon className="ac-toast__icon" name={icon ?? TONE_ICON[tone]} size={17} />
      <span className="ac-toast__copy">
        {title ? <span className="ac-toast__title">{title}</span> : null}
        <span className="ac-toast__message">{message}</span>
      </span>
      {action ? (
        <button className="ac-toast__action" onClick={action.onClick} type="button">
          {action.label}
        </button>
      ) : null}
      {onDismiss ? (
        <button aria-label={text('toastDismiss', 'Dismiss notification')} className="ac-toast__close" onClick={onDismiss} type="button">
          <Icon name="x" size={14} />
        </button>
      ) : null}
    </div>
  );
}

export type ToastView = ToastProps & { id: string };

/** Fixed bottom-right stack that holds the active toasts. */
export function ToastViewport({
  toasts,
  onDismiss,
  className,
}: {
  toasts: readonly ToastView[];
  onDismiss: (id: string) => void;
  className?: string;
}) {
  return (
    <div
      aria-label={text('toastRegion', 'Notifications')}
      aria-live="polite"
      className={cx('ac-toast__viewport', className)}
      role="region"
    >
      {toasts.map(({ id, ...toast }) => (
        <Toast {...toast} key={id} onDismiss={() => onDismiss(id)} />
      ))}
    </div>
  );
}
