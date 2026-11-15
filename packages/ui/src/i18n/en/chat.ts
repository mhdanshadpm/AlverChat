/* English catalog for the conversation surfaces (spec 7.1-7.3). Flat keys, no prefix. */
export default {
  /* Mic refusals, spoken once from the composer and once from the recorder. */
  micFallback: 'Your chosen microphone is not connected, so the default one recorded this.',
  micDenied: 'Microphone access is off, so voice notes cannot be recorded.',

  /* The three transfer actions a file bubble offers, named by what they do. */
  pauseTransfer: 'Pause transfer',
  resumeTransfer: 'Resume transfer',

  /* What the bubble and the message menu announce out loud: the multi-select checkbox, a
     reaction chip, the media tiles, and the quick-reaction row. Every one is an accessible
     name, so a screen reader says this English — it is as user-facing as a visible label. */
  selectMessageAria: 'Select message',
  selectMessageFrom: 'Select message from {{time}}',
  reactionChip: '{{count}} reaction: {{names}}',
  reactionChipMany: '{{count}} reactions: {{names}}',
  reactWith: 'React with {{emoji}}',
  moreReactions: 'More reactions',
  playVoiceNote: 'Play voice note',
  pauseVoiceNote: 'Pause voice note',
  openImage: 'Open image',
  openImageNumbered: 'Open image {{number}}',
  playVideo: 'Play video',

  /* The four states a file bubble leads with, plus the sending percentage. */
  filePaused: 'Paused',
  fileFailed: 'Failed',
  fileComplete: 'Complete',
  fileSending: 'Sending…',
  fileSendingPercent: 'Sending… {{percent}}%',

  // List pane
  workspace: 'YOUR WORKSPACE',
  messages: 'Messages',
  searchChats: 'Search chats, people and messages…',
  filterAll: 'All',
  filterUnread: 'Unread',
  filterGroups: 'Groups',
  filterMuted: 'Muted',
  filterGroup: 'Filter chats',
  pinnedSection: 'Pinned',
  chatsSection: 'Chats',
  inMessages: 'In messages',
  newConversation: 'New conversation',
  newGroup: 'New group',
  noChatsTitle: 'No chats yet',
  noChatsBody: 'Chats appear once AlverChat finds someone on this local network.',
  noMatchesTitle: 'No matches found',
  noMatchesBody: 'Try another name, keyword or message text.',
  searchingLan: 'Searching the local network',
  peersDiscovered: '{{count}} peer found',
  peersDiscoveredMany: '{{count}} peers found',
  readyCount: '{{count}} ready',

  cancel: 'Cancel',
  close: 'Close',
  back: 'Back',
  delete: 'Delete',
  previousMatch: 'Previous match',
  nextMatch: 'Next match',
  messageMenu: 'Message actions',
  confirmDeleteMine: 'This message disappears from this device only.',
  confirmDeleteEveryone: 'Everyone loses this message. It cannot be undone.',
  shortcutSend: 'Enter to send · Shift+Enter for a new line',
  retry: 'Retry',

  cancelTransfer: 'Cancel transfer',

  // Row state

  typingNow: 'Typing…',
  tapToOpen: 'Open chat',

  // Row context menu
  pinChat: 'Pin chat',
  unpinChat: 'Unpin chat',
  muteChat: 'Mute notifications',
  unmuteChat: 'Unmute notifications',
  markUnreadAction: 'Mark as unread',
  clearHistory: 'Clear chat history',
  deleteChatAction: 'Delete chat',

  confirmClear: 'Delete every message in this chat on this device?',
  confirmDeleteChat: 'Remove this chat and its history from this device?',

  // Conversation header
  voiceCall: 'Voice call',
  videoCall: 'Video call',
  chatDetails: 'Chat details',
  searchInChat: 'Search in chat',
  moreOptions: 'More options',

  encryptionOn: 'End-to-end encrypted on this local network',
  encryptionOff: 'Not encrypted — set the network passphrase',
  peerUnavailable: 'Peer unavailable on the local network',
  peerUnavailableBody: 'They are not answering discovery right now, so messages cannot be delivered.',

  // Message list
  today: 'Today',
  yesterday: 'Yesterday',
  newMessages: 'New messages',
  scrollToBottom: 'Scroll to latest message',
  unreadCount: '{{count}} unread',
  noMessagesTitle: 'No messages yet',
  noMessagesBody: 'Send the first message to start this conversation.',
  pinnedBar: 'Pinned message',
  unpin: 'Unpin',

  deletedMessage: 'This message was deleted.',

  // Bubble details

  forwardedLabel: 'Forwarded',
  editedTag: '(edited)',
  deliverySending: 'Sending',
  deliveryDelivered: 'Delivered',
  deliveryRead: 'Read',
  downloadFile: 'Download {{name}}',

  // Message context menu
  react: 'React',
  reply: 'Reply',
  edit: 'Edit',
  copy: 'Copy',
  forward: 'Forward',
  pin: 'Pin',
  unpinMessage: 'Unpin',
  star: 'Star',
  unstar: 'Remove star',

  deleteForMe: 'Delete for me',
  deleteForEveryone: 'Delete for everyone',
  selectMessage: 'Select',
  cannotEditExpired: 'The edit window of {{minutes}} minutes has passed.',
  cannotEditOthers: 'Only your own messages can be edited.',

  // Multi-select toolbar
  selectedCount: '{{count}} selected',
  forwardTo: 'Forward',
  deleteSelected: 'Delete',
  clearSelection: 'Clear selection',

  // Composer
  writeMessage: 'Write a message…',
  editMessage: 'Edit your message…',
  attach: 'Attach',
  attachPhoto: 'Photo or video',
  attachFile: 'File or document',
  attachScreenshot: 'Screenshot',
  attachVoice: 'Voice note',
  screenshotUnavailable: 'Screen capture is not available in this window.',
  screenshotDenied: 'Screen capture was cancelled.',
  screenshotFailed: 'Could not capture the screen.',
  emoji: 'Emoji',
  recordVoice: 'Record a voice note',

  send: 'Send',
  recordingFor: 'Recording {{time}} — release to send',
  cancelRecording: 'Cancel recording',
  sendRecording: 'Send voice note',
  editBannerTitle: 'Edit message',
  editBannerBody: 'Update your message',
  replyBannerTitle: 'Reply to {{name}}',
  closeBanner: 'Cancel',
  fileTooLarge: '{{name}} is bigger than the {{limit}} MB limit.',
  fileSent: '{{name}} is transferring.',

  messageCopied: 'Message copied.',
  deletedForMe: 'Message deleted for you.',
  deletedForEveryone: 'Message deleted for everyone.',
  starred: 'Starred.',
  unstarred: 'Star removed.',
  pinnedToast: 'Message pinned.',
  unpinnedToast: 'Message unpinned.',
  unreadMarked: 'Marked as unread.',
  chatCleared: 'Chat history cleared on this device.',
  chatDeleted: 'Chat deleted from this device.',
  featureSoon: 'This action is unavailable in the current build.',

  forwardedCount: '{{count}} message forwarded',
  forwardedCountMany: '{{count}} messages forwarded',
  forwardPick: 'Choose where to forward',
  forwardSend: 'Forward',

  // In-chat search
  searchResults: '{{count}} match',
  searchResultsMany: '{{count}} matches',
  searchPlaceholder: 'Search this chat…',
  noResultsTitle: 'Nothing found',

  // Details pane
  detailsTitle: 'Chat details',
  mediaTab: 'Media',
  filesTab: 'Files',
  linksTab: 'Links',
  starredTab: 'Starred',
  pinnedTab: 'Pinned',
  membersTab: 'Members',
  emptySharedTitle: 'No shared content',
  emptySharedBody: 'Shared media, files and pinned messages appear here.',
  sharedFiles: 'Shared files',
  storageUsed: '{{used}} used',
  muteToggle: 'Mute notifications',
  encryptionTitle: 'Encryption',

  exportChat: 'Export this chat',
  exportReady: 'Export written to {{path}}',
  linkCopied: 'Link copied.',
  copyFingerprint: 'Copy my fingerprint',
  confirm: 'Confirm',
  confirmLeaveGroup: 'You will stop receiving this group until you are added again.',
  leftGroup: 'You left the group.',
  clearChat: 'Clear chat',
  deleteChat: 'Delete chat',
  groupAdmin: 'Admin',
  groupYou: 'You',
  leaveGroup: 'Leave group',
  addMembers: 'Add members',

  // Keyboard shortcut hints
  shortcutReply: 'R',
  shortcutCopy: 'Ctrl+C',
  shortcutEdit: 'Ctrl+E',
  shortcutForward: 'Ctrl+F',

};
