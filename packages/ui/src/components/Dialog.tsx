import type { ReactNode } from 'react';
import * as DialogPrimitive from '@radix-ui/react-dialog';
import { Icon, cx } from './Icon';

export type DialogSize = 'sm' | 'md' | 'lg' | 'xl';

export type DialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: ReactNode;
  description?: ReactNode;
  children?: ReactNode;
  /** Footer slot; pass Buttons. */
  footer?: ReactNode;
  /** Spread the footer buttons to both ends, as the new-group dialog does. */
  footerLayout?: 'end' | 'split';
  size?: DialogSize;
  /** Hide the header close glyph (still dismissible with Esc). */
  hideClose?: boolean;
  closeLabel?: string;
  /** Clicking the scrim dismisses the dialog. */
  dismissOnInteractOutside?: boolean;
  /** Keep the dialog mounted so its content can animate or hold form state. */
  forceMount?: boolean;
  className?: string;
};

/**
 * Modal built on `@radix-ui/react-dialog`, so focus trapping, `Esc`, the
 * scrim click and the `aria-modal`/labelled-by wiring all come from Radix.
 */
export function Dialog({
  open,
  onOpenChange,
  title,
  description,
  children,
  footer,
  footerLayout = 'end',
  size = 'md',
  hideClose = false,
  closeLabel = 'Close dialog',
  dismissOnInteractOutside = true,
  forceMount = false,
  className,
}: DialogProps) {
  const mount = forceMount ? { forceMount: true as const } : {};

  return (
    <DialogPrimitive.Root onOpenChange={onOpenChange} open={open}>
      <DialogPrimitive.Portal {...mount}>
        <DialogPrimitive.Overlay className="ac-dialog__overlay" {...mount} />
        <DialogPrimitive.Content
          className={cx('ac-dialog__content', size !== 'md' && `ac-dialog__content--${size}`, className)}
          {...mount}
          onInteractOutside={(event) => {
            if (!dismissOnInteractOutside) event.preventDefault();
          }}
          onOpenAutoFocus={(event) => {
            // Let the first text field take focus instead of the whole sheet.
            const node = event.target as HTMLElement;
            const field = node.querySelector<HTMLElement>(
              'input:not([disabled]), textarea:not([disabled]), select:not([disabled])',
            );
            if (!field) return;
            event.preventDefault();
            field.focus();
          }}
          onPointerDownOutside={(event) => {
            if (!dismissOnInteractOutside) event.preventDefault();
          }}
          role="dialog"
        >
          {/* Radix exposes no Header/Footer primitives; these are plain slots
              that keep the documented `dialog` labelling intact. */}
          <div className="ac-dialog__header">
            <DialogPrimitive.Title className="ac-dialog__title">{title}</DialogPrimitive.Title>
            {hideClose ? null : (
              <DialogPrimitive.Close asChild>
                <button aria-label={closeLabel} className="ac-iconbtn ac-iconbtn--sm" type="button">
                  <Icon name="x" size={16} />
                </button>
              </DialogPrimitive.Close>
            )}
          </div>
          <div className="ac-dialog__body">
            {description ? (
              <DialogPrimitive.Description className="ac-dialog__description">
                {description}
              </DialogPrimitive.Description>
            ) : null}
            {children}
          </div>
          {footer ? (
            <div className={cx('ac-dialog__footer', footerLayout === 'split' && 'ac-dialog__footer--split')}>
              {footer}
            </div>
          ) : null}
        </DialogPrimitive.Content>
      </DialogPrimitive.Portal>
    </DialogPrimitive.Root>
  );
}
