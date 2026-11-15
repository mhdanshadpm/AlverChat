import type { ReactNode } from 'react';
import * as MenuPrimitive from '@radix-ui/react-dropdown-menu';
import { ns, registerNamespace } from '../i18n';
import chatCatalog from '../i18n/en/chat';
import { Icon, cx, type IconName } from './Icon';
import { MENU_WIDTH, measureMenuHeight, placeMenu, type MenuPlacement } from './logic';

registerNamespace('chat', chatCatalog);

const text = ns('chat');

export { placeMenu, measureMenuHeight, MENU_WIDTH };
export type { MenuPlacement };

export type ContextMenuItem = {
  id: string;
  label: string;
  icon?: IconName;
  /** Right-aligned 12px muted column, e.g. "Ctrl+C". */
  shortcut?: string;
  /** Red text + red hover fill, per the "Danger action" spec. */
  danger?: boolean;
  disabled?: boolean;
  /** Shows a leading check glyph, as on the "Select" item. */
  checked?: boolean;
  /** Nested entries; opens with the 150ms ease-out submenu animation. */
  submenu?: readonly ContextMenuEntry[];
  onSelect?: () => void;
};

export type ContextMenuEntry =
  | ({ kind: 'item' } & ContextMenuItem)
  | { kind: 'separator'; id: string }
  | { kind: 'label'; id: string; label: string };

export type ContextMenuProps = {
  /** Fully controlled: the owner decides when the menu exists. */
  open: boolean;
  onClose: () => void;
  /** Pointer position the menu was invoked at, in viewport coordinates. */
  anchor: { x: number; y: number } | null;
  items: readonly ContextMenuEntry[];
  /** Quick-emoji row above the items, as on the message menu. */
  reactions?: readonly string[];
  onReaction?: (emoji: string) => void;
  /** Trailing "⋯" button that opens the full picker. */
  onMoreReactions?: () => void;
  /** Accessible name for the menu. */
  label?: string;
  /** Viewport size; defaults to the live window. */
  viewportWidth?: number;
  viewportHeight?: number;
  /** Override the 220px spec width. */
  width?: number;
  className?: string;
};

const MORE_EMOJI = '⋯';

function countEntries(entries: readonly ContextMenuEntry[]) {
  let items = 0;
  let separators = 0;
  for (const entry of entries) {
    if (entry.kind === 'item') items += 1;
    else if (entry.kind === 'separator') separators += 1;
  }
  return { items, separators };
}

function viewportSize(): { width: number; height: number } {
  if (typeof window === 'undefined') return { width: 0, height: 0 };
  return { width: window.innerWidth, height: window.innerHeight };
}

/**
 * The custom context-menu system from the boards: 220px wide, 8px radius,
 * 40px items, 20px Lucide glyphs, 14px Medium labels, a muted shortcut column,
 * dividers, disabled items at 50% opacity, danger items in red, submenus and an
 * optional emoji header row.
 *
 * Built on `@radix-ui/react-dropdown-menu`, which supplies the keyboard model
 * the spec calls for: ArrowUp/ArrowDown to move, Home/End to jump, Enter or
 * Space to select, Esc to dismiss, and typeahead on the first letter.
 *
 * Placement is ours, not Radix's: `placeMenu` clamps the anchor to the viewport
 * and Radix collision avoidance is disabled, so the exported helper is the
 * single source of truth and always agrees with `placeContextMenu`.
 */
export function ContextMenu({
  open,
  onClose,
  anchor,
  items,
  reactions,
  onReaction,
  onMoreReactions,
  label = 'Context menu',
  viewportWidth,
  viewportHeight,
  width = MENU_WIDTH,
  className,
}: ContextMenuProps) {
  if (!open || !anchor) return null;

  const fallback = viewportSize();
  const vw = viewportWidth ?? fallback.width;
  const vh = viewportHeight ?? fallback.height;
  const { items: itemCount, separators } = countEntries(items);
  const estimated = measureMenuHeight(itemCount, {
    reactionRow: Boolean(reactions && reactions.length > 0),
    separators,
  });
  const placement = placeMenu(anchor.x, anchor.y, vw, vh, width, estimated);

  return (
    <MenuPrimitive.Root
      modal={false}
      onOpenChange={(next) => {
        if (!next) onClose();
      }}
      open
    >
      {/* Radix's dropdown-menu has no standalone Anchor, so the invisible
          Trigger is parked at the clamped point and the Content is positioned
          from it with collision avoidance turned off. */}
      <span
        className="ac-menu__anchor"
        style={{ left: `${placement.left}px`, top: `${placement.top}px` }}
      >
        <MenuPrimitive.Trigger asChild tabIndex={-1}>
          <span aria-hidden className="ac-menu__anchor-dot" />
        </MenuPrimitive.Trigger>
      </span>
      <MenuPrimitive.Portal>
        <MenuPrimitive.Content
          align="start"
          aria-label={label}
          avoidCollisions={false}
          className={cx('ac-menu__content', placement.flipped && 'ac-menu__content--flipped', className)}
          loop={false}
          onCloseAutoFocus={(event) => {
            event.preventDefault();
          }}
          side="bottom"
          sideOffset={0}
        >
          {reactions && reactions.length > 0 ? (
            <div className="ac-menu__reactions">
              {reactions.map((emoji) => (
                <button
                  aria-label={text('reactWith', 'React with {{emoji}}', { emoji })}
                  className="ac-menu__reaction"
                  key={emoji}
                  onClick={() => {
                    onReaction?.(emoji);
                    onClose();
                  }}
                  type="button"
                >
                  {emoji}
                </button>
              ))}
              {onMoreReactions ? (
                <button
                  aria-label={text('moreReactions', 'More reactions')}
                  className="ac-menu__reaction ac-menu__reaction--more"
                  onClick={onMoreReactions}
                  type="button"
                >
                  {MORE_EMOJI}
                </button>
              ) : null}
            </div>
          ) : null}
          {items.map((entry) => (
            <MenuEntry key={entry.id} entry={entry} onClose={onClose} />
          ))}
        </MenuPrimitive.Content>
      </MenuPrimitive.Portal>
    </MenuPrimitive.Root>
  );
}

function MenuEntry({ entry, onClose }: { entry: ContextMenuEntry; onClose: () => void }) {
  if (entry.kind === 'separator') {
    return <MenuPrimitive.Separator className="ac-menu__separator" />;
  }
  if (entry.kind === 'label') {
    return <MenuPrimitive.Label className="ac-menu__label">{entry.label}</MenuPrimitive.Label>;
  }

  const glyph = entry.icon ? <Icon name={entry.icon} size={20} /> : null;
  const shortcut = entry.shortcut ? (
    <span className="ac-menu__shortcut">{entry.shortcut}</span>
  ) : null;

  if (entry.submenu) {
    return (
      <MenuPrimitive.Sub>
        <MenuPrimitive.SubTrigger
          className={cx('ac-menu__subtrigger', entry.danger && 'ac-menu__item--danger')}
          disabled={entry.disabled}
        >
          {glyph}
          {entry.label}
          {shortcut}
          <Icon className="ac-menu__subarrow" name="chevronRight" size={16} />
        </MenuPrimitive.SubTrigger>
        <MenuPrimitive.Portal>
          <MenuPrimitive.SubContent alignOffset={-6} className="ac-menu__subcontent" sideOffset={4}>
            {entry.submenu.map((child) => (
              <MenuEntry entry={child} key={child.id} onClose={onClose} />
            ))}
          </MenuPrimitive.SubContent>
        </MenuPrimitive.Portal>
      </MenuPrimitive.Sub>
    );
  }

  return (
    <MenuPrimitive.Item
      className={cx('ac-menu__item', entry.danger && 'ac-menu__item--danger')}
      data-checked={entry.checked ? 'true' : undefined}
      disabled={entry.disabled}
      onSelect={() => {
        entry.onSelect?.();
        onClose();
      }}
      textValue={entry.label}
    >
      {entry.checked ? <Icon className="ac-menu__checked" name="check" size={16} /> : null}
      {glyph}
      {entry.label}
      {shortcut}
    </MenuPrimitive.Item>
  );
}

/** Convenience for callers that only have plain items. */
export function contextMenuItems(items: readonly ContextMenuItem[]): ContextMenuEntry[] {
  return items.map((item) => ({ ...item, kind: 'item' as const }));
}

export type ContextMenuTriggerProps = {
  children: ReactNode;
  /** Called with the pointer coordinates of the context-menu event. */
  onOpen: (point: { x: number; y: number }) => void;
  className?: string;
};

/** Wraps a region so right-clicking it reports the pointer position. */
export function ContextMenuTriggerRegion({ children, onOpen, className }: ContextMenuTriggerProps) {
  return (
    <span
      className={cx('ac-menu__trigger', className)}
      onContextMenu={(event) => {
        event.preventDefault();
        event.stopPropagation();
        onOpen({ x: event.clientX, y: event.clientY });
      }}
    >
      {children}
    </span>
  );
}
