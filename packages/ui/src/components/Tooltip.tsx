import {
  useCallback,
  useId,
  useRef,
  useState,
  type ReactNode,
} from 'react';
import { cx } from './Icon';

export type TooltipSide = 'top' | 'bottom' | 'left' | 'right';

export type TooltipProps = {
  /** The description shown in the bubble. */
  content: ReactNode;
  children: ReactNode;
  side?: TooltipSide;
  /** ms before the bubble appears; 0 shows it immediately. */
  delay?: number;
  /** Hide the bubble, e.g. while a context menu is open. */
  disabled?: boolean;
  /** Allow multi-line bubbles instead of a single nowrap line. */
  wrap?: boolean;
  /** Keeps the bubble visible for hover-card style usage. */
  open?: boolean;
  className?: string;
};

/**
 * Hand-rolled tooltip (no Radix tooltip package is installed).
 * The bubble is `role="tooltip"` and wired to the trigger with
 * `aria-describedby`, so it is announced on focus as well as on hover.
 */
export function Tooltip({
  content,
  children,
  side = 'top',
  delay = 200,
  disabled = false,
  wrap = false,
  open,
  className,
}: TooltipProps) {
  const id = useId();
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const [hovered, setHovered] = useState(false);
  const visible = !disabled && (open ?? hovered);

  const clear = useCallback(() => {
    if (timer.current !== null) {
      clearTimeout(timer.current);
      timer.current = null;
    }
  }, []);

  const show = useCallback(() => {
    clear();
    if (delay <= 0) {
      setHovered(true);
      return;
    }
    timer.current = setTimeout(() => {
      timer.current = null;
      setHovered(true);
    }, delay);
  }, [clear, delay]);

  const hide = useCallback(() => {
    clear();
    setHovered(false);
  }, [clear]);

  return (
    <span
      className={cx('ac-tooltip', className)}
      onBlur={hide}
      onFocus={show}
      onPointerEnter={show}
      onPointerLeave={hide}
    >
      <span aria-describedby={visible ? id : undefined} className="ac-tooltip__trigger">
        {children}
      </span>
      {visible ? (
        <span
          className={cx(
            'ac-tooltip__bubble',
            side !== 'top' && `ac-tooltip__bubble--${side}`,
            wrap && 'ac-tooltip__bubble--wrap',
          )}
          id={id}
          role="tooltip"
        >
          {content}
        </span>
      ) : null}
    </span>
  );
}
