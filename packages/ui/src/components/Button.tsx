import { forwardRef, type ButtonHTMLAttributes, type ReactNode } from 'react';
import { Icon, cx, type IconName } from './Icon';

export type ButtonVariant = 'primary' | 'secondary' | 'danger' | 'ghost';
export type ButtonSize = 'sm' | 'md' | 'lg';

export type ButtonProps = Omit<ButtonHTMLAttributes<HTMLButtonElement>, 'children'> & {
  variant?: ButtonVariant;
  size?: ButtonSize;
  /** Leading glyph; the label stays required for accessibility. */
  icon?: IconName;
  iconSide?: 'left' | 'right';
  /** Shows a spinner and blocks interaction. */
  loading?: boolean;
  /** Stretch to the container width. */
  block?: boolean;
  children?: ReactNode;
};

/**
 * The three button styles on the Settings board: Primary (filled violet),
 * Secondary (outlined surface) and Danger (red outline that fills on hover).
 */
export const Button = forwardRef<HTMLButtonElement, ButtonProps>(function Button(
  {
    variant = 'primary',
    size = 'md',
    icon,
    iconSide = 'left',
    loading = false,
    block = false,
    disabled,
    className,
    children,
    type = 'button',
    ...rest
  },
  ref,
) {
  const glyphSize = size === 'sm' ? 14 : size === 'lg' ? 18 : 16;
  return (
    <button
      {...rest}
      ref={ref}
      type={type}
      aria-disabled={disabled || loading || undefined}
      aria-busy={loading || undefined}
      disabled={disabled ?? loading}
      className={cx(
        'ac-btn',
        `ac-btn--${variant}`,
        size !== 'md' && `ac-btn--${size}`,
        block && 'ac-btn--block',
        className,
      )}
    >
      {loading ? (
        <Icon name="loader" size={glyphSize} spin />
      ) : icon && iconSide === 'left' ? (
        <Icon name={icon} size={glyphSize} />
      ) : null}
      {children ? <span className="ac-btn__label">{children}</span> : null}
      {icon && iconSide === 'right' && !loading ? <Icon name={icon} size={glyphSize} /> : null}
    </button>
  );
});
