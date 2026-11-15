import './components.css';

/* ===== Icon (the only lucide-react entry point) ===== */
export { Icon, ICONS, ICON_NAMES, cx } from './Icon';
export type { IconMap, IconName, IconProps } from './Icon';

/* ===== Buttons ===== */
export { Button } from './Button';
export type { ButtonProps, ButtonSize, ButtonVariant } from './Button';
export { IconButton } from './IconButton';
export type { IconButtonProps, IconButtonSize, IconButtonVariant } from './IconButton';

/* ===== Identity ===== */
export { Avatar, AvatarStack, PresenceDot } from './Avatar';
export { PeerAvatar } from './PeerAvatar';
export type { PeerAvatarProps } from './PeerAvatar';
export type {
  AvatarProps,
  AvatarSize,
  AvatarStackProps,
} from './Avatar';
export { Badge, BadgeCount } from './Badge';
export type { BadgeCountProps, BadgeProps, BadgeTone } from './Badge';

/* ===== Form controls ===== */
export { Input } from './Input';
export type { InputProps, InputSize } from './Input';
export { SearchInput } from './SearchInput';
export type { SearchInputProps } from './SearchInput';
export { Toggle, UncontrolledToggle } from './Toggle';
export type { ToggleProps, UncontrolledToggleProps } from './Toggle';
export { Slider } from './Slider';
export type { SliderProps } from './Slider';
export { Select, toSelectOptions } from './Select';
export type { SelectOption, SelectProps } from './Select';
export { Segmented } from './Segmented';
export type { SegmentedOption, SegmentedProps } from './Segmented';
export { Tabs } from './Tabs';
export type { TabsProps, TabItem } from './Tabs';
export { ListGroup, ListRow } from './ListRow';
export type { ListGroupProps, ListRowProps } from './ListRow';

/* ===== Settings-board controls (added for the Settings screen) ===== */
export { ThemeCards } from './ThemeCards';
export type { ThemeCardsProps, ThemeChoice } from './ThemeCards';
export { WeekdayPicker } from './WeekdayPicker';
export type { WeekdayPickerProps } from './WeekdayPicker';
export { HotkeyRecorder } from './HotkeyRecorder';
export type { HotkeyRecorderProps } from './HotkeyRecorder';

/* ===== Overlays ===== */
export { Dialog } from './Dialog';
export type { DialogProps, DialogSize } from './Dialog';
export { Toast, ToastViewport } from './Toast';
export type { ToastProps, ToastTone, ToastView } from './Toast';
export { Tooltip } from './Tooltip';
export type { TooltipProps, TooltipSide } from './Tooltip';
export {
  ContextMenu,
  ContextMenuTriggerRegion,
  contextMenuItems,
  MENU_WIDTH,
  measureMenuHeight,
  placeMenu,
} from './ContextMenu';
export type {
  ContextMenuEntry,
  ContextMenuItem,
  ContextMenuProps,
  ContextMenuTriggerProps,
  MenuPlacement,
} from './ContextMenu';

/* ===== Feedback ===== */
export { Banner } from './Banner';
export type { BannerProps, BannerTone } from './Banner';
export { ProgressBar } from './ProgressBar';
export type { ProgressBarProps, ProgressTone } from './ProgressBar';
export { Skeleton, SkeletonList } from './Skeleton';
export type { SkeletonProps, SkeletonVariant } from './Skeleton';
export { EmptyState } from './EmptyState';
export type { EmptyStateProps } from './EmptyState';
export { Waveform, WaveformPlayButton } from './Waveform';
export type { WaveformProps } from './Waveform';

/* ===== Conversation ===== */
export { MessageBubble } from './MessageBubble';
export type { BubbleMedia, BubbleReply, MessageBubbleProps, MessageVariant } from './MessageBubble';

/* ===== Pure logic (also what components.test.tsx covers) ===== */
export {
  AVATAR_PALETTE,
  MAX_COMPOSITE_MEMBERS,
  MENU_EDGE_MARGIN,
  MENU_ITEM_HEIGHT,
  MENU_PADDING,
  MENU_REACTION_ROW_HEIGHT,
  MENU_SEPARATOR_HEIGHT,
  PRESENCE_STATES,
  avatarPaletteFor,
  clampPercent,
  compositeMembers,
  deliveryTicks,
  fileKindFor,
  fileTransferDetail,
  fileTransferLabel,
  formatCount,
  formatDuration,
  formatFileSize,
  groupReactions,
  initialsFor,
  nextIndexOf,
  presenceForPeer,
  reactionTooltip,
  statusLineFor,
  transferPercent,
  typeaheadIndex,
  withinEditWindow,
  waveformBars,
  waveformProgress,
} from './logic';
export type {
  AvatarMember,
  AvatarPaletteColor,
  DeliveryState,
  FileGlyph,
  FileTransferState,
  PresenceState,
  ReactionGroup,
  ReactionInput,
  TickSpec,
} from './logic';
