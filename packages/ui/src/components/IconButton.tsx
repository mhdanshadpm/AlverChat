import { forwardRef, type ButtonHTMLAttributes } from 'react';
import { Icon, cx, type IconName } from './Icon';

export type IconButtonVariant = 'plain' | 'soft' | 'raised' | 'danger';
export type IconButtonSize = 'sm' | 'md' | 'lg';

export type IconButtonProps = Omit<ButtonHTMLAttributes<HTMLButtonElement>, 'children'> & {
  name: IconName;
  /** Required: an icon-only control must always announce itself. */
  label: string;
  variant?: IconButtonVariant;
  size?: IconButtonSize;
  /** Renders the selected/active state, e.g. the mute button in a chat head. */
  active?: boolean;
  /** Overrides the 20px default when the glyph needs to be smaller. */
  iconSize?: number;
  tooltip?: string;
};

/** Bare glyph button used in toolbars, row trailing controls and dialog headers. */
export const IconButton = forwardRef<HTMLButtonElement, IconButtonProps>(function IconButton(
  {
    name,
    label,
    variant = 'plain',
    size = 'md',
    active = false,
    iconSize,
    tooltip,
    className,
    type = 'button',
    disabled,
    ...rest
  },
  ref,
) {
  const glyph = iconSize ?? (size === 'sm' ? 15 : size === 'lg' ? 22 : 18);
  return (
    <button
      {...rest}
      ref={ref}
      type={type}
      aria-label={label}
      aria-pressed={active || undefined}
      title={tooltip}
      disabled={disabled}
      className={cx(
        'ac-iconbtn',
        `ac-iconbtn--${size}`,
        variant !== 'plain' && `ac-iconbtn--${variant}`,
        active && 'ac-iconbtn--active',
        className,
      )}
    >
      <Icon name={name} size={glyph} />
    </button>
  );
});
