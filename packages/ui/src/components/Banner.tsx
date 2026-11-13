import type { ReactNode } from 'react';
import { Icon, cx, type IconName } from './Icon';

export type BannerTone = 'neutral' | 'info' | 'success' | 'warning' | 'danger';

export type BannerProps = {
  tone?: BannerTone;
  title?: ReactNode;
  message?: ReactNode;
  icon?: IconName;
  /** Primary affordance, e.g. "Open guide". */
  action?: ReactNode;
  /** Secondary text button next to the copy, e.g. "View all pinned messages". */
  link?: ReactNode;
  onDismiss?: () => void;
  dismissLabel?: string;
  /** Full-bleed strip pinned under a header instead of a rounded card. */
  inline?: boolean;
  className?: string;
};

const TONE_ICON: Record<BannerTone, IconName> = {
  neutral: 'info',
  info: 'info',
  success: 'shieldCheck',
  warning: 'alertCircle',
  danger: 'alertCircle',
};

/**
 * Inline notice: the LAN-connected banner on the People screen and the
 * announcement strip under a conversation header.
 */
export function Banner({
  tone = 'neutral',
  title,
  message,
  icon,
  action,
  link,
  onDismiss,
  dismissLabel = 'Dismiss',
  inline = false,
  className,
}: BannerProps) {
  return (
    <div
      className={cx('ac-banner', tone !== 'neutral' && `ac-banner--${tone}`, inline && 'ac-banner--inline', className)}
      role={tone === 'danger' || tone === 'warning' ? 'alert' : 'status'}
    >
      <span className="ac-banner__icon">
        <Icon name={icon ?? TONE_ICON[tone]} size={18} />
      </span>
      <span className="ac-banner__body">
        {title ? <span className="ac-banner__title">{title}</span> : null}
        {message ? <span className="ac-banner__message">{message}</span> : null}
        {link ? <span className="ac-banner__link">{link}</span> : null}
      </span>
      {action ? <span className="ac-banner__actions">{action}</span> : null}
      {onDismiss ? (
        <button aria-label={dismissLabel} className="ac-banner__close" onClick={onDismiss} type="button">
          <Icon name="x" size={15} />
        </button>
      ) : null}
    </div>
  );
}
