import { cx } from './Icon';

export type SkeletonVariant =
  | 'text'
  | 'title'
  | 'circle'
  | 'avatar'
  | 'rect'
  | 'bubble'
  | 'row';

export type SkeletonProps = {
  variant?: SkeletonVariant;
  /** Number of stacked lines for `text`. */
  lines?: number;
  className?: string;
};

/** Shimmering placeholder. Always hidden from assistive tech. */
export function Skeleton({ variant = 'text', lines = 1, className }: SkeletonProps) {
  if (variant === 'text' && lines > 1) {
    return (
      <span aria-hidden className={cx('ac-skeleton--stack', className)}>
        {Array.from({ length: lines }, (_, index) => (
          <span
            className="ac-skeleton ac-skeleton--text"
            key={index}
            style={{ width: index === lines - 1 ? '60%' : '100%' }}
          />
        ))}
      </span>
    );
  }
  return <span aria-hidden className={cx('ac-skeleton', `ac-skeleton--${variant}`, className)} />;
}

/** Convenience placeholder for a whole chat list while peers load. */
export function SkeletonList({ rows = 5, className }: { rows?: number; className?: string }) {
  return (
    <span aria-hidden className={cx('ac-skeleton--stack', className)}>
      {Array.from({ length: rows }, (_, index) => (
        <Skeleton key={index} variant="row" />
      ))}
    </span>
  );
}
