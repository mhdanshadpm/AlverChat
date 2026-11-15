import { useCallback, useState } from 'react';
import { Icon, cx, type IconName } from './Icon';
import {
  avatarPaletteFor,
  compositeMembers,
  initialsFor,
  type AvatarMember,
  type AvatarPaletteColor,
  type PresenceState,
} from './logic';

export type AvatarSize = 24 | 40 | 56 | 'lg';

export type AvatarProps = {
  /** Full name — drives the initials fallback and the default palette slot. */
  name?: string;
  /** Overrides the derived initials. */
  initials?: string;
  /** Photo URL. Falls back to initials if it fails to load. */
  src?: string;
  /** Forces one of the 8 palette colours instead of hashing the name. */
  color?: AvatarPaletteColor;
  size?: AvatarSize;
  presence?: PresenceState;
  /** Group conversation: filled gradient disc with the people glyph. */
  group?: boolean;
  /** Palette colour for the group glyph variant. */
  groupColor?: AvatarPaletteColor;
  /**
   * Composite mode — renders up to four members in the 2x2 cluster from the
   * groups board. Takes precedence over `src`, `group` and `name`.
   */
  members?: readonly AvatarMember[];
  /** Accessible name; defaults to `name` or "Group avatar". */
  label?: string;
  /** Decorative: the surrounding control already names the identity. */
  decorative?: boolean;
  /** Glyph shown instead of initials, e.g. for the "Everyone" group. */
  glyph?: IconName;
  className?: string;
};

const SIZE_CLASS: Record<AvatarSize, string> = {
  24: 'ac-avatar--24',
  40: 'ac-avatar--40',
  56: 'ac-avatar--56',
  lg: 'ac-avatar--lg',
};

/**
 * Avatar with photo, 8-colour initials fallback, presence dot, group variant
 * and the composite 2x2 cluster for group conversations.
 */
export function Avatar({
  name,
  initials,
  src,
  color,
  size = 40,
  presence,
  group = false,
  groupColor,
  members,
  label,
  glyph,
  decorative,
  className,
}: AvatarProps) {
  const [failed, setFailed] = useState(false);
  const onError = useCallback(() => setFailed(true), []);
  const palette = color ?? avatarPaletteFor(name);
  const tone = group ? (groupColor ?? palette) : palette;
  const text = initials ?? initialsFor(name);
  const composite = members && members.length > 0 ? compositeMembers(members) : null;

  const labelText = decorative
    ? undefined
    : label ?? name ?? (group || composite ? 'Group avatar' : undefined);

  const shared = {
    'aria-label': labelText,
    // An avatar with no name is decorative (a bubble's sender is already named
    // in the row), so it must not surface as an unlabeled image to assistive tech.
    'aria-hidden': labelText ? undefined : true,
    className: cx(
      'ac-avatar',
      SIZE_CLASS[size],
      composite ? 'ac-avatar--composite' : `ac-avatar--${tone}`,
      group && !composite && 'ac-avatar--group',
      className,
    ),
    role: labelText ? ('img' as const) : undefined,
  };

  if (composite) {
    return (
      <span {...shared}>
        <span className="ac-avatar__grid">
          {composite.map((member, index) => (
            <span
              className={cx(
                'ac-avatar__cell',
                `ac-avatar__cell--${member.color ?? avatarPaletteFor(member.name)}`,
              )}
              key={`${member.name ?? 'member'}-${index}`}
            >
              {initialsFor(member.name)}
              {member.src ? <img alt="" className="ac-avatar__cell-img" src={member.src} /> : null}
            </span>
          ))}
        </span>
        {presence ? <PresenceDot state={presence} /> : null}
      </span>
    );
  }

  const showPhoto = Boolean(src) && !failed;

  return (
    <span {...shared}>
      {showPhoto ? (
        <img alt="" className="ac-avatar__img" onError={onError} src={src} />
      ) : group ? (
        <Icon name={glyph ?? 'users'} size={size === 24 ? 13 : size === 40 ? 18 : 22} />
      ) : (
        <span aria-hidden>{text}</span>
      )}
      {presence ? <PresenceDot state={presence} /> : null}
    </span>
  );
}

/** Presence dot: online (green), away (amber), busy (red), offline (grey). */
export function PresenceDot({ state }: { state: PresenceState }) {
  return <i aria-hidden className={cx('ac-avatar__dot', `ac-avatar__dot--${state}`)} />;
}

export type AvatarStackProps = {
  members: readonly AvatarMember[];
  size?: AvatarSize;
  /** Max avatars before the trailing "+n" chip appears. */
  max?: number;
  className?: string;
};

/** Overlapping avatar stack used in the group info panel header. */
export function AvatarStack({ members, size = 24, max = 4, className }: AvatarStackProps) {
  const shown = members.slice(0, max);
  const extra = members.length - shown.length;
  return (
    <span className={cx('ac-avatar__stack', className)}>
      {shown.map((member, index) => (
        <Avatar
          color={member.color}
          key={`${member.name ?? 'member'}-${index}`}
          name={member.name}
          presence={member.presence}
          size={size}
          src={member.src}
        />
      ))}
      {extra > 0 ? <span className="ac-avatar__stack-more">+{extra}</span> : null}
    </span>
  );
}

export type { AvatarMember, AvatarPaletteColor, PresenceState };
