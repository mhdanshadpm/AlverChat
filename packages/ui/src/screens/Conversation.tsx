import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore, type CSSProperties } from 'react';
import { useVirtualizer } from '@tanstack/react-virtual';
import type {
  AppSettings,
  Attachment,
  ChatMessage,
  Peer,
  PinnedMessage,
  PlatformBridge,
  ReactionTally,
} from '../../../bridge/src';
import {
  attachmentPreview,
  attachmentPreviewVersion,
  clearAttachmentPreviews,
  loadAttachmentPreview,
  subscribeAttachmentPreviews,
  type AttachmentPreviewSource,
} from '../attachmentPreviews';
import {
  Avatar,
  ContextMenu,
  EmptyState,
  Icon,
  IconButton,
  MessageBubble,
  PeerAvatar,
  SearchInput,
  Tooltip,
  avatarPaletteFor,
  cx,
  formatDuration,
  initialsFor,
  presenceForPeer,
  statusLineFor,
  withinEditWindow,
  type ContextMenuEntry,
  type DeliveryState,
  type FileTransferState,
  type MessageVariant,
  type ReactionInput,
} from '../components';
import { ns, registerNamespace } from '../i18n';
import chatCatalog from '../i18n/en/chat';
import { applySinkId, openMedia } from '../media';
import { authorName, messageTime, type ToastTone } from './shared';
import { VOICE_SAMPLE_RATE, downsample, mergeSamples, wavBlob } from './voice';
import './chat.css';

registerNamespace('chat', chatCatalog);
const text = ns('chat');

const QUICK_REACTIONS = ['👍', '❤️', '😂', '🎉', '😮', '😢'];

const EMOJI = [
  '😀', '😁', '😂', '🤣', '😊', '😍', '🤔', '😐',
  '😴', '😎', '🥳', '😭', '😡', '🤯', '🤝', '👏',
  '👍', '👎', '🙏', '💪', '👀', '🧠', '❤️', '💔',
  '🔥', '✨', '🎉', '🚀', '💡', '☕', '🍕', '🌙',
  '✅', '❌', '⚠️', '📌', '📎', '📁', '🖼️', '🎙️',
];

const IMAGE_TYPES = ['png', 'jpg', 'jpeg', 'webp', 'gif'];
const VIDEO_TYPES = ['mp4', 'webm', 'mov', 'mkv'];

export type ConversationProps = {
  bridge: PlatformBridge;
  peer: Peer | null;
  chatId: string;
  peers: Peer[];
  settings: AppSettings;
  /** My peer id — decides who may edit or unsend a message. */
  myId: string;
  detailsOpen: boolean;
  onBack: () => void;
  onToggleDetails: (open: boolean) => void;
  onCall: (kind: 'voice' | 'video') => void;
  toast: (message: string, tone?: ToastTone) => void;
  /** Bumped by the shell on `messages-changed`, so the thread refreshes in place. */
  revision?: number;
  className?: string;
};

type ReactionMap = Record<string, ReactionTally[]>;

type Row =
  | { kind: 'day'; id: string; label: string }
  | { kind: 'unread'; id: string; count: number }
  | { kind: 'message'; id: string; message: ChatMessage; grouped: boolean };

function dayKey(ms: number): string {
  const date = new Date(ms);
  return `${date.getFullYear()}-${date.getMonth()}-${date.getDate()}`;
}

function midnight(ms: number): number {
  const date = new Date(ms);
  date.setHours(0, 0, 0, 0);
  return date.getTime();
}

function dayLabel(ms: number): string {
  const days = Math.round((midnight(Date.now()) - midnight(ms)) / 86_400_000);
  if (days <= 0) return text('today', 'Today');
  if (days === 1) return text('yesterday', 'Yesterday');
  return new Date(ms).toLocaleDateString([], { weekday: 'short', month: 'short', day: 'numeric' });
}

function stampOf(message: ChatMessage): number {
  return message.timestampMs ?? Number.NaN;
}

function deliveryOf(message: ChatMessage): DeliveryState {
  if (message.deliveryStatus === 'pending') return 'sending';
  return message.read ? 'read' : 'delivered';
}

function transferOf(attachment: Attachment | undefined): FileTransferState | undefined {
  if (!attachment) return undefined;
  if (attachment.state === 'done') return 'complete';
  if (attachment.state === 'paused') return 'paused';
  if (attachment.state === 'failed' || attachment.state === 'cancelled') return 'failed';
  return 'sending';
}

function extensionOf(name: string): string {
  const dot = name.lastIndexOf('.');
  return dot > 0 ? name.slice(dot + 1).toLowerCase() : '';
}

/**
 * Which bubble a file message is. A picture is only a picture once its bytes are on this
 * machine and something can draw them, so the resolved preview — not the row — decides
 * whether the bubble is media or a file chip; a video never resolves one, because neither
 * webview will hand us a frame to show for it.
 */
function variantOf(message: ChatMessage, preview: string | null | undefined, attachment?: Attachment): MessageVariant {
  if (message.kind === 'deleted') return 'deleted';
  if (message.kind === 'event') return 'text';
  if (message.kind !== 'file') return 'text';
  const kind =
    attachment?.kind ??
    (IMAGE_TYPES.includes(extensionOf(message.fileName ?? ''))
      ? 'image'
      : VIDEO_TYPES.includes(extensionOf(message.fileName ?? ''))
        ? 'video'
        : 'file');
  if (kind === 'audio') return 'voice';
  if (kind === 'image' && preview) return 'image';
  return 'file';
}

function isMine(message: ChatMessage, myId: string): boolean {
  return Boolean(message.mine) || message.sender === myId;
}

function readAsDataUrl(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result));
    reader.onerror = () => reject(reader.error ?? new Error('read failed'));
    reader.readAsDataURL(blob);
  });
}

type VoiceCapture = {
  context: AudioContext;
  source: MediaStreamAudioSourceNode;
  processor: ScriptProcessorNode;
  silence: GainNode;
  stream: MediaStream;
  sampleRate: number;
};

export function Conversation({
  bridge,
  peer,
  chatId,
  peers,
  settings,
  myId,
  detailsOpen,
  onBack,
  onToggleDetails,
  onCall,
  toast,
  revision,
  className,
}: ConversationProps) {
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [attachments, setAttachments] = useState<Attachment[]>([]);
  const [reactions, setReactions] = useState<ReactionMap>({});
  const [pinned, setPinned] = useState<PinnedMessage[]>([]);
  const [starred, setStarred] = useState<string[]>([]);
  const [firstUnread, setFirstUnread] = useState<string | null>(null);
  const [unreadCount, setUnreadCount] = useState(0);

  const [draft, setDraft] = useState('');
  const [editing, setEditing] = useState<{ id: string; text: string } | null>(null);
  const [replyTo, setReplyTo] = useState<ChatMessage | null>(null);
  const [selected, setSelected] = useState<string[]>([]);
  const [menu, setMenu] = useState<{ message: ChatMessage; x: number; y: number } | null>(null);
  const [confirm, setConfirm] = useState<{ message: ChatMessage; everyone: boolean } | null>(null);
  const [forwarding, setForwarding] = useState(false);

  const [findOpen, setFindOpen] = useState(false);
  const [findQuery, setFindQuery] = useState('');
  const [findHits, setFindHits] = useState<string[]>([]);
  const [findIndex, setFindIndex] = useState(0);
  const [pinIndex, setPinIndex] = useState(0);

  const [attachOpen, setAttachOpen] = useState(false);
  const [emojiOpen, setEmojiOpen] = useState(false);
  const [recordingMs, setRecordingMs] = useState<number | null>(null);
  const [atBottom, setAtBottom] = useState(true);

  const scrollRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const pictureInput = useRef<HTMLInputElement>(null);
  const fileInput = useRef<HTMLInputElement>(null);
  const recorderRef = useRef<VoiceCapture | null>(null);
  const chunksRef = useRef<Float32Array[]>([]);
  const tickRef = useRef<number | null>(null);
  const startedAtRef = useRef(0);

  const peerName = peer?.name ?? chatId;
  const isGroup = Boolean(peer?.group);

  /** The ids storage leaves in `sender` and in a reaction tally, read back as names. */
  const nameOf = useCallback(
    (id: string) =>
      authorName(id, peers, {
        id: myId,
        name: settings.displayName.trim() || text('groupYou', 'You'),
      }),
    [myId, peers, settings.displayName],
  );

  const reload = useCallback(async () => {
    const [history, files, pins] = await Promise.all([
      bridge.listMessages(chatId),
      bridge.attachmentsFor(chatId).catch(() => [] as Attachment[]),
      bridge.pinnedMessages(chatId).catch(() => [] as PinnedMessage[]),
    ]);
    const sorted = [...history].sort((left, right) => stampOf(left) - stampOf(right));
    setMessages(sorted);
    setAttachments(files);
    setPinned(pins);
    const tallies = await Promise.all(
      sorted.map(message =>
        bridge
          .reactionsFor(message.id)
          .then(list => [message.id, list] as const)
          .catch(() => [message.id, [] as ReactionTally[]] as const),
      ),
    );
    setReactions(Object.fromEntries(tallies.filter(([, list]) => list.length > 0)));
    const stars = await bridge.starredMessages(chatId).catch(() => [] as ChatMessage[]);
    setStarred(stars.map(entry => entry.id));
  }, [bridge, chatId]);

  useEffect(() => {
    let alive = true;
    // Another thread's pictures are not this one's, and holding both would mean the
    // cache grows with every chat opened in a long session.
    clearAttachmentPreviews();
    setMessages([]);
    setSelected([]);
    setDraft('');
    setEditing(null);
    setReplyTo(null);
    setPinIndex(0);
    setFirstUnread(null);
    setUnreadCount(0);
    void (async () => {
      // The unread span has to be read before opening the chat clears it.
      const before = await bridge.listMessages(chatId).catch(() => [] as ChatMessage[]);
      const unseen = before.filter(
        message => !message.mine && message.kind !== 'deleted' && !message.read,
      );
      if (alive && unseen.length > 0) {
        setFirstUnread(unseen[0].id);
        setUnreadCount(unseen.length);
      }
      if (!alive) return;
      void bridge.markAsRead(chatId).catch(() => undefined);
      await reload();
    })();
    return () => {
      alive = false;
    };
  }, [bridge, chatId, reload]);

  // The mount effect already loaded this chat; only a *changed* revision means new traffic.
  const seenRevision = useRef(revision);
  useEffect(() => {
    if (revision === undefined || seenRevision.current === revision) return;
    seenRevision.current = revision;
    void reload();
    // A message that arrives into an open chat is read as it appears.
    void bridge.markAsRead(chatId).catch(() => undefined);
  }, [bridge, chatId, reload, revision]);

  const byId = useMemo(() => new Map(messages.map(message => [message.id, message])), [messages]);
  const attachmentByMessage = useMemo(() => {
    const map = new Map<string, Attachment>();
    for (const file of attachments) if (file.messageId) map.set(file.messageId, file);
    return map;
  }, [attachments]);

  const rows = useMemo<Row[]>(() => {
    const out: Row[] = [];
    let lastDay = '';
    let previous: ChatMessage | null = null;
    for (const message of messages) {
      const ms = stampOf(message);
      const key = Number.isNaN(ms) ? '' : dayKey(ms);
      if (key && key !== lastDay) {
        out.push({ kind: 'day', id: `day-${key}`, label: dayLabel(ms) });
        lastDay = key;
        previous = null;
      }
      if (message.id === firstUnread && unreadCount > 0) {
        out.push({ kind: 'unread', id: `unread-${message.id}`, count: unreadCount });
      }
      const gap = Number.isNaN(ms) || Number.isNaN(stampOf(previous ?? message)) ? 0 : ms - (previous?.timestampMs ?? ms);
      const grouped =
        previous !== null &&
        previous.kind === 'text' &&
        message.kind === 'text' &&
        previous.sender === message.sender &&
        !message.replyTo &&
        gap < 5 * 60_000;
      out.push({ kind: 'message', id: message.id, message, grouped });
      previous = message;
    }
    return out;
  }, [messages, firstUnread, unreadCount]);

  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: index => (rows[index]?.kind === 'message' ? 64 : 34),
    initialOffset: Number.MAX_SAFE_INTEGER,
    overscan: 8,
  });

  const jumpToLatest = useCallback(
    (smooth = true) => {
      if (rows.length === 0) return;
      virtualizer.scrollToIndex(rows.length - 1, { align: 'end', behavior: smooth ? 'smooth' : 'auto' });
    },
    [rows.length, virtualizer],
  );

  const newestId = messages[messages.length - 1]?.id;
  useEffect(() => {
    if (atBottom) jumpToLatest(false);
  }, [newestId, atBottom, jumpToLatest]);

  const onScroll = useCallback(() => {
    const element = scrollRef.current;
    if (!element) return;
    setAtBottom(element.scrollHeight - element.scrollTop - element.clientHeight < 80);
  }, []);

  /* ===== find in chat ==================================================== */

  useEffect(() => {
    const needle = findQuery.trim().toLowerCase();
    if (needle.length < 2) {
      setFindHits([]);
      return;
    }
    setFindHits(messages.filter(message => message.text.toLowerCase().includes(needle)).map(message => message.id));
    setFindIndex(0);
  }, [findQuery, messages]);

  const step = useCallback(
    (delta: number) => {
      if (findHits.length === 0) return;
      const next = (findIndex + delta + findHits.length) % findHits.length;
      setFindIndex(next);
      const index = rows.findIndex(row => row.id === findHits[next]);
      if (index >= 0) virtualizer.scrollToIndex(index, { align: 'center' });
    },
    [findHits, findIndex, rows, virtualizer],
  );

  /* ===== typing indicator ================================================ */

  const typingPulse = useRef(0);
  const typingTimer = useRef<number | null>(null);

  const stopTyping = useCallback(() => {
    if (typingTimer.current !== null) window.clearTimeout(typingTimer.current);
    typingTimer.current = null;
    if (typingPulse.current === 0) return;
    typingPulse.current = 0;
    void bridge.setTyping(chatId, false).catch(() => undefined);
  }, [bridge, chatId]);

  const announceTyping = useCallback(() => {
    const now = Date.now();
    // One frame per three seconds while the user keeps typing; the peer's flag
    // lapses on its own if this window dies mid-sentence.
    if (now - typingPulse.current > 3_000) {
      typingPulse.current = now;
      void bridge.setTyping(chatId, true).catch(() => undefined);
    }
    if (typingTimer.current !== null) window.clearTimeout(typingTimer.current);
    typingTimer.current = window.setTimeout(stopTyping, 4_000);
  }, [bridge, chatId, stopTyping]);

  useEffect(() => stopTyping, [stopTyping]);

  /* ===== mutations ======================================================= */

  const send = useCallback(async () => {
    const body = draft.trim();
    if (!body) return;
    setDraft('');
    if (inputRef.current) inputRef.current.style.height = 'auto';
    try {
      await bridge.sendMessage(chatId, body, replyTo?.id);
      stopTyping();
      setReplyTo(null);
      await reload();
      requestAnimationFrame(() => jumpToLatest());
    } catch (error) {
      setDraft(body);
      toast(String(error), 'danger');
    }
  }, [bridge, chatId, draft, jumpToLatest, reload, replyTo, stopTyping, toast]);

  const commitEdit = useCallback(async () => {
    if (!editing) return;
    const body = editing.text.trim();
    setEditing(null);
    try {
      const ok = await bridge.editMessageLocal(editing.id, body);
      if (!ok) toast(text('cannotEditOthers', 'Only your own messages can be edited.'), 'neutral');
      await reload();
    } catch (error) {
      toast(String(error), 'danger');
    }
  }, [bridge, editing, reload, toast]);

  const react = useCallback(
    async (message: ChatMessage, emoji: string) => {
      try {
        const tally = await bridge.react(message.id, emoji);
        setReactions(current => ({ ...current, [message.id]: tally }));
      } catch (error) {
        toast(String(error), 'danger');
      }
    },
    [bridge, toast],
  );

  const togglePin = useCallback(
    async (message: ChatMessage) => {
      const wasPinned = pinned.some(entry => entry.messageId === message.id);
      try {
        setPinned(await bridge.pinMessage(chatId, message.id, !wasPinned));
        toast(wasPinned ? text('unpinnedToast', 'Message unpinned.') : text('pinnedToast', 'Message pinned.'));
      } catch (error) {
        toast(String(error), 'danger');
      }
    },
    [bridge, chatId, pinned, toast],
  );

  const toggleStar = useCallback(
    async (message: ChatMessage) => {
      const on = !starred.includes(message.id);
      try {
        await bridge.starMessage(message.id, on);
        setStarred(current => (on ? [...current, message.id] : current.filter(id => id !== message.id)));
        toast(on ? text('starred', 'Starred.') : text('unstarred', 'Star removed.'));
      } catch (error) {
        toast(String(error), 'danger');
      }
    },
    [bridge, starred, toast],
  );

  const remove = useCallback(
    async (ids: string[], everyone: boolean) => {
      try {
        for (const id of ids) {
          if (everyone) await bridge.deleteForEveryone(id);
          else await bridge.deleteMessageLocal(id);
        }
        setSelected([]);
        await reload();
        toast(
          everyone
            ? text('deletedForEveryone', 'Message deleted for everyone.')
            : text('deletedForMe', 'Message deleted for you.'),
        );
      } catch (error) {
        toast(String(error), 'danger');
      }
    },
    [bridge, reload, toast],
  );

  const copy = useCallback(
    async (message: ChatMessage) => {
      try {
        await navigator.clipboard.writeText(message.text);
        toast(text('messageCopied', 'Message copied.'));
      } catch {
        toast(text('featureSoon', 'This action is unavailable in the current build.'), 'neutral');
      }
    },
    [toast],
  );

  /* ===== attachments ===================================================== */

  const pickFile = useCallback(
    async (file: File | undefined) => {
      if (!file) return;
      const limit = Math.max(1, settings.maxFileSizeMb) * 1024 * 1024;
      if (file.size > limit) {
        toast(
          text('fileTooLarge', '{{name}} is bigger than the {{limit}} MB limit.', {
            name: file.name,
            limit: settings.maxFileSizeMb,
          }),
          'danger',
        );
        return;
      }
      try {
        const dataUrl = await readAsDataUrl(file);
        await bridge.sendFile(chatId, file.name, dataUrl);
        toast(text('fileSent', '{{name}} is transferring.', { name: file.name }));
        await reload();
        requestAnimationFrame(() => jumpToLatest());
      } catch (error) {
        toast(String(error), 'danger');
      }
    },
    [bridge, chatId, jumpToLatest, reload, settings.maxFileSizeMb, toast],
  );

  const control = useCallback(
    async (attachmentId: string, action: 'pause' | 'resume' | 'retry' | 'cancel') => {
      try {
        await bridge.controlTransfer(attachmentId, action);
        await reload();
      } catch (error) {
        toast(String(error), 'danger');
      }
    },
    [bridge, reload, toast],
  );

  const voiceSource = useCallback((id: string) => bridge.attachmentData(id), [bridge]);
  const voiceError = useCallback((error: unknown) => toast(String(error), 'danger'), [toast]);

  /* ===== voice note ====================================================== */

  const stopTimer = useCallback(() => {
    if (tickRef.current) window.clearInterval(tickRef.current);
    tickRef.current = null;
  }, []);

  const startRecording = useCallback(async () => {
    if (recordingMs !== null) return;
    setAttachOpen(false);
    const AudioContextCtor =
      window.AudioContext ??
      (window as typeof window & { webkitAudioContext?: typeof AudioContext }).webkitAudioContext;
    if (!AudioContextCtor || !navigator.mediaDevices?.getUserMedia) {
      toast(text('featureSoon', 'This action is unavailable in the current build.'), 'neutral');
      return;
    }
    try {
      const opened = await openMedia(navigator.mediaDevices, {
        audio: {
          deviceId: settings.microphone && settings.microphone !== 'default' ? { exact: settings.microphone } : undefined,
          echoCancellation: settings.echoCancellation,
          noiseSuppression: settings.noiseSuppression,
        },
        video: false,
      });
      if (opened.usedDefault) {
        toast(text('micFallback', 'Your chosen microphone is not connected, so the default one recorded this.'), 'neutral');
      }
      const stream = opened.stream;
      const context = new AudioContextCtor();
      await context.resume();
      const source = context.createMediaStreamSource(stream);
      const processor = context.createScriptProcessor(4096, 1, 1);
      // A silent sink keeps the processor pulling samples without echoing the mic.
      const silence = context.createGain();
      silence.gain.value = 0;
      chunksRef.current = [];
      processor.onaudioprocess = event => {
        chunksRef.current.push(new Float32Array(event.inputBuffer.getChannelData(0)));
      };
      source.connect(processor);
      processor.connect(silence);
      silence.connect(context.destination);
      recorderRef.current = { context, source, processor, silence, stream, sampleRate: context.sampleRate };
      const started = Date.now();
      startedAtRef.current = started;
      setRecordingMs(0);
      stopTimer();
      tickRef.current = window.setInterval(() => setRecordingMs(Date.now() - started), 200);
    } catch {
      toast(text('micDenied', 'Microphone access is off, so voice notes cannot be recorded.'), 'danger');
    }
  }, [recordingMs, settings.echoCancellation, settings.microphone, settings.noiseSuppression, stopTimer, toast]);

  const stopRecording = useCallback(
    async (cancel: boolean) => {
      const capture = recorderRef.current;
      recorderRef.current = null;
      const elapsedMs = startedAtRef.current ? Date.now() - startedAtRef.current : 0;
      startedAtRef.current = 0;
      stopTimer();
      setRecordingMs(null);
      if (!capture) return;
      capture.processor.onaudioprocess = null;
      capture.source.disconnect();
      capture.processor.disconnect();
      capture.silence.disconnect();
      capture.stream.getTracks().forEach(track => track.stop());
      void capture.context.close();
      if (cancel) {
        chunksRef.current = [];
        return;
      }
      const samples = mergeSamples(chunksRef.current);
      chunksRef.current = [];
      if (samples.length === 0) {
        toast(text('micDenied', 'Microphone access is off, so voice notes cannot be recorded.'), 'danger');
        return;
      }
      const recordedMs = Math.round((samples.length / capture.sampleRate) * 1000);
      const blob = wavBlob(downsample(samples, capture.sampleRate, VOICE_SAMPLE_RATE), VOICE_SAMPLE_RATE);
      try {
        const dataUrl = await readAsDataUrl(blob);
        await bridge.sendFile(chatId, 'voice-note.wav', dataUrl, recordedMs || elapsedMs || undefined);
        await reload();
        requestAnimationFrame(() => jumpToLatest());
      } catch (error) {
        toast(String(error), 'danger');
      }
    },
    [bridge, chatId, jumpToLatest, reload, stopTimer, toast],
  );

  useEffect(() => {
    return () => {
      if (tickRef.current) window.clearInterval(tickRef.current);
      const capture = recorderRef.current;
      if (capture) {
        capture.source.disconnect();
        capture.processor.disconnect();
        capture.silence.disconnect();
        capture.stream.getTracks().forEach(track => track.stop());
        void capture.context.close();
      }
    };
  }, []);

  /* ===== selection ======================================================= */

  const selectionOn = selected.length > 0;
  const toggleSelected = useCallback((messageId: string, on: boolean) => {
    setSelected(current => (on ? [...new Set([...current, messageId])] : current.filter(id => id !== messageId)));
  }, []);

  const forwardTo = useCallback(
    async (chatIds: string[]) => {
      setForwarding(false);
      try {
        const count = await bridge.forwardMessages(selected, chatIds);
        toast(
          count === 1
            ? text('forwardedCount', '{{count}} message forwarded', { count })
            : text('forwardedCountMany', '{{count}} messages forwarded', { count }),
        );
        setSelected([]);
        await reload();
      } catch (error) {
        toast(String(error), 'danger');
      }
    },
    [bridge, reload, selected, toast],
  );

  /* ===== per-message menu =============================================== */

  const beginEdit = useCallback(
    (message: ChatMessage) => {
      const withinWindow = withinEditWindow(stampOf(message), Date.now(), settings.editWindowMinutes);
      if (!withinWindow) {
        toast(
          text('cannotEditExpired', 'The edit window of {{minutes}} minutes has passed.', {
            minutes: settings.editWindowMinutes,
          }),
          'neutral',
        );
        return;
      }
      setEditing({ id: message.id, text: message.text });
      requestAnimationFrame(() => inputRef.current?.focus());
    },
    [settings.editWindowMinutes, toast],
  );

  const menuFor = useCallback(
    (message: ChatMessage): ContextMenuEntry[] => {
      const mine = isMine(message, myId);
      const withinWindow = withinEditWindow(stampOf(message), Date.now(), settings.editWindowMinutes);
      const isPinned = pinned.some(entry => entry.messageId === message.id);
      return [
        {
          kind: 'item',
          id: 'reply',
          icon: 'reply',
          label: text('reply', 'Reply'),
          shortcut: text('shortcutReply', 'R'),
          onSelect: () => {
            setReplyTo(message);
            requestAnimationFrame(() => inputRef.current?.focus());
          },
        },
        {
          kind: 'item',
          id: 'copy',
          icon: 'copy',
          label: text('copy', 'Copy'),
          shortcut: text('shortcutCopy', 'Ctrl+C'),
          onSelect: () => void copy(message),
        },
        {
          kind: 'item',
          id: 'edit',
          icon: 'pencil',
          label: text('edit', 'Edit'),
          shortcut: text('shortcutEdit', 'Ctrl+E'),
          disabled: !mine,
          onSelect: () => {
            if (!mine) toast(text('cannotEditOthers', 'Only your own messages can be edited.'), 'neutral');
            else beginEdit(message);
          },
        },
        {
          kind: 'item',
          id: 'forward',
          icon: 'forward',
          label: text('forward', 'Forward'),
          shortcut: text('shortcutForward', 'Ctrl+F'),
          onSelect: () => {
            setSelected([message.id]);
            setForwarding(true);
          },
        },
        {
          kind: 'item',
          id: 'pin',
          icon: isPinned ? 'pinOff' : 'pin',
          label: isPinned ? text('unpinMessage', 'Unpin') : text('pin', 'Pin'),
          onSelect: () => void togglePin(message),
        },
        {
          kind: 'item',
          id: 'star',
          icon: 'star',
          label: starred.includes(message.id) ? text('unstar', 'Remove star') : text('star', 'Star'),
          onSelect: () => void toggleStar(message),
        },
        {
          kind: 'item',
          id: 'select',
          icon: 'check',
          label: text('selectMessage', 'Select'),
          onSelect: () => toggleSelected(message.id, true),
        },
        { kind: 'separator', id: 'sep' },
        {
          kind: 'item',
          id: 'delete-me',
          icon: 'trash',
          danger: true,
          label: text('deleteForMe', 'Delete for me'),
          onSelect: () => setConfirm({ message, everyone: false }),
        },
        {
          kind: 'item',
          id: 'delete-all',
          icon: 'ban',
          danger: true,
          label: text('deleteForEveryone', 'Delete for everyone'),
          disabled: !mine || !withinWindow,
          onSelect: () => setConfirm({ message, everyone: true }),
        },
      ];
    },
    [beginEdit, copy, myId, pinned, settings.editWindowMinutes, starred, togglePin, toggleSelected, toggleStar, toast],
  );

  const typing = Boolean(peer?.typing);
  const subtitle = typing
    ? text('typingNow', 'Typing…')
    : (peer && statusLineFor(peer)) ||
      (peer?.online
        ? text('encryptionOn', 'End-to-end encrypted on this local network')
        : text('peerUnavailable', 'Peer unavailable on the local network'));
  const activePin = pinned.length > 0 ? pinned[pinIndex % pinned.length] : null;

  return (
    <section aria-label={peerName} className={cx('ac-convo', className)}>
      <header className="ac-convo__head">
        <IconButton iconSize={18} label={text('back', 'Back')} name="chevronLeft" onClick={onBack} />
        <button className="ac-convo__who" onClick={() => onToggleDetails(!detailsOpen)} type="button">
          <PeerAvatar
            group={isGroup}
            initials={peer?.initials || initialsFor(peerName)}
            name={peerName}
            peerId={isGroup ? null : peer?.id}
            presence={peer ? presenceForPeer(peer) : 'offline'}
            size={40}
            source={bridge}
          />
          <span className="ac-convo__names">
            <strong>{peerName}</strong>
            <small className={cx(typing && 'is-typing')}>
              {!peer?.online && !typing ? <Icon name="wifi" size={12} /> : null}
              {subtitle}
            </small>
          </span>
        </button>
        <div className="ac-convo__tools">
          <IconButton
            active={findOpen}
            iconSize={18}
            label={text('searchInChat', 'Search in chat')}
            name="search"
            onClick={() => setFindOpen(open => !open)}
          />
          <IconButton iconSize={18} label={text('voiceCall', 'Voice call')} name="phoneCall" onClick={() => onCall('voice')} />
          <IconButton iconSize={18} label={text('videoCall', 'Video call')} name="video" onClick={() => onCall('video')} />
          <IconButton
            active={detailsOpen}
            iconSize={18}
            label={text('chatDetails', 'Chat details')}
            name="info"
            onClick={() => onToggleDetails(!detailsOpen)}
          />
        </div>
      </header>

      {activePin ? (
        <div className="ac-convo__strip">
          <span className="ac-convo__strip-label">
            <Icon name="pin" size={13} />
            {text('pinnedBar', 'Pinned message')}
          </span>
          <button
            className="ac-convo__strip-text"
            onClick={() => {
              const index = rows.findIndex(row => row.id === activePin.messageId);
              if (index >= 0) virtualizer.scrollToIndex(index, { align: 'center' });
            }}
            type="button"
          >
            {activePin.text || text('deletedMessage', 'This message was deleted.')}
          </button>
          {pinned.length > 1 ? (
            <button className="ac-convo__strip-count" onClick={() => setPinIndex(index => index + 1)} type="button">
              {pinIndex % pinned.length + 1}/{pinned.length}
            </button>
          ) : null}
          <IconButton
            iconSize={15}
            label={text('unpin', 'Unpin')}
            name="pinOff"
            onClick={() => void bridge.pinMessage(chatId, activePin.messageId, false).then(setPinned)}
          />
        </div>
      ) : null}

      {findOpen ? (
        <div className="ac-convo__find">
          <SearchInput
            autoFocus
            onChange={setFindQuery}
            placeholder={text('searchPlaceholder', 'Search this chat…')}
            value={findQuery}
          />
          <span className="ac-convo__strip-count">
            {findHits.length > 0
              ? findHits.length === 1
                ? text('searchResults', '{{count}} match', { count: findHits.length })
                : text('searchResultsMany', '{{count}} matches', { count: findHits.length })
              : findQuery.trim().length >= 2
                ? text('noResultsTitle', 'Nothing found')
                : ''}
          </span>
          <IconButton disabled={findHits.length === 0} iconSize={16} label={text('previousMatch', 'Previous match')} name="chevronUp" onClick={() => step(-1)} />
          <IconButton disabled={findHits.length === 0} iconSize={16} label={text('nextMatch', 'Next match')} name="chevronDown" onClick={() => step(1)} />
          <IconButton iconSize={16} label={text('close', 'Close')} name="x" onClick={() => setFindOpen(false)} />
        </div>
      ) : null}

      {selectionOn ? (
        <div className="ac-convo__selection">
          <strong>{text('selectedCount', '{{count}} selected', { count: selected.length })}</strong>
          <div className="ac-convo__selection-actions">
            <IconButton iconSize={17} label={text('forwardTo', 'Forward')} name="forward" onClick={() => setForwarding(true)} />
            <IconButton
              iconSize={17}
              label={text('deleteSelected', 'Delete')}
              name="trash"
              onClick={() => {
                const first = byId.get(selected[0]);
                if (first) setConfirm({ message: first, everyone: false });
              }}
            />
            <IconButton iconSize={17} label={text('clearSelection', 'Clear selection')} name="x" onClick={() => setSelected([])} />
          </div>
        </div>
      ) : null}

      <div className="ac-convo__scroll" onScroll={onScroll} ref={scrollRef}>
        {messages.length === 0 ? (
          <EmptyState
            compact
            icon="messageCircle"
            message={
              peer?.online
                ? text('noMessagesBody', 'Send the first message to start this conversation.')
                : text('peerUnavailableBody', 'They are not answering discovery right now, so messages cannot be delivered.')
            }
            title={text('noMessagesTitle', 'No messages yet')}
          />
        ) : (
          <div style={{ height: virtualizer.getTotalSize(), position: 'relative' }}>
            {virtualizer.getVirtualItems().map(item => {
              const row = rows[item.index];
              if (!row) return null;
              const shared = {
                index: item.index,
                style: {
                  left: 0,
                  position: 'absolute' as const,
                  top: 0,
                  transform: `translateY(${item.start}px)`,
                  width: '100%',
                },
                wrapRef: virtualizer.measureElement,
              };
              if (row.kind === 'day') {
                return (
                  <div data-index={shared.index} key={row.id} ref={shared.wrapRef} style={shared.style}>
                    {row.label}
                  </div>
                );
              }
              if (row.kind === 'unread') {
                return (
                  <div data-index={shared.index} key={row.id} ref={shared.wrapRef} style={shared.style}>
                    <span className="ac-convo__unread">
                      {text('newMessages', 'New messages')} ·{' '}
                      {text('unreadCount', '{{count}} unread', { count: row.count })}
                    </span>
                  </div>
                );
              }
              return (
                <MessageRow
                  {...shared}
                  attachment={attachmentByMessage.get(row.message.id)}
                  byId={byId}
                  grouped={row.grouped}
                  key={row.id}
                  message={row.message}
                  mine={isMine(row.message, myId)}
                  nameOf={nameOf}
                  onContextMenu={point => setMenu({ message: row.message, ...point })}
                  onOpen={id => void bridge.openAttachment(id).catch(() => undefined)}
                  onReact={emoji => void react(row.message, emoji)}
                  onReply={() => setReplyTo(row.message)}
                  onSelect={selectionOn ? on => toggleSelected(row.message.id, on) : undefined}
                  onTransfer={control}
                  onVoiceError={voiceError}
                  onVoiceSource={voiceSource}
                  previewSource={bridge}
                  reactions={reactions[row.message.id] ?? []}
                  selected={selectionOn ? selected.includes(row.message.id) : undefined}
                  speaker={settings.speaker}
                  starred={starred.includes(row.message.id)}
                />
              );
            })}
          </div>
        )}

        {!atBottom && messages.length > 0 ? (
          <button className="ac-convo__jump" onClick={() => jumpToLatest()} type="button">
            <Icon name="chevronDown" size={15} />
            {text('scrollToBottom', 'Scroll to latest message')}
          </button>
        ) : null}
      </div>

      <footer className="ac-composer">
        {replyTo ? (
          <div className="ac-composer__banner">
            <Icon name="reply" size={16} />
            <span className="ac-composer__banner-body">
              <strong>{text('replyBannerTitle', 'Reply to {{name}}', { name: replyTo.sender })}</strong>
              <span>{replyTo.text}</span>
            </span>
            <IconButton iconSize={16} label={text('closeBanner', 'Cancel')} name="x" onClick={() => setReplyTo(null)} />
          </div>
        ) : null}
        {editing ? (
          <div className="ac-composer__banner">
            <Icon name="pencil" size={16} />
            <span className="ac-composer__banner-body">
              <strong>{text('editBannerTitle', 'Edit message')}</strong>
              <span>{text('editBannerBody', 'Update your message')}</span>
            </span>
            <IconButton iconSize={16} label={text('closeBanner', 'Cancel')} name="x" onClick={() => setEditing(null)} />
          </div>
        ) : null}

        {recordingMs !== null ? (
          <div className="ac-composer__record">
            <Icon name="mic" size={16} />
            <span className="ac-composer__record-time">
              {text('recordingFor', 'Recording {{time}} — release to send', {
                time: formatDuration(Math.round(recordingMs / 1000)),
              })}
            </span>
            <div className="ac-composer__selection-actions">
              <button
                aria-label={text('cancelRecording', 'Cancel recording')}
                className="ac-composer__send"
                onClick={() => void stopRecording(true)}
                type="button"
              >
                <Icon name="trash" size={17} />
              </button>
              <button
                aria-label={text('sendRecording', 'Send voice note')}
                className="ac-composer__send"
                onClick={() => void stopRecording(false)}
                type="button"
              >
                <Icon name="send" size={17} />
              </button>
            </div>
          </div>
        ) : (
          <div className="ac-composer__field">
            <Tooltip content={text('attach', 'Attach')} delay={200}>
              <IconButton active={attachOpen} iconSize={18} label={text('attach', 'Attach')} name="plus" onClick={() => setAttachOpen(open => !open)} />
            </Tooltip>
            <textarea
              aria-label={editing ? text('editMessage', 'Edit your message…') : text('writeMessage', 'Write a message…')}
              className="ac-composer__input"
              onChange={event => {
                setDraft(editing ? '' : event.target.value);
                if (editing) setEditing({ ...editing, text: event.target.value });
                else if (event.target.value.trim()) announceTyping();
                event.target.style.height = 'auto';
                event.target.style.height = `${Math.min(event.target.scrollHeight, 148)}px`;
              }}
              onKeyDown={event => {
                if (event.key === 'Enter' && !event.shiftKey) {
                  event.preventDefault();
                  if (editing) void commitEdit();
                  else void send();
                }
                if (event.key === 'Escape') {
                  setEditing(null);
                  setReplyTo(null);
                }
              }}
              placeholder={editing ? text('editMessage', 'Edit your message…') : text('writeMessage', 'Write a message…')}
              ref={inputRef}
              rows={1}
              value={editing ? editing.text : draft}
            />
            <Tooltip content={text('emoji', 'Emoji')} delay={200}>
              <IconButton active={emojiOpen} iconSize={18} label={text('emoji', 'Emoji')} name="smile" onClick={() => setEmojiOpen(open => !open)} />
            </Tooltip>
            {editing ? (
              <button
                aria-label={text('confirm', 'Confirm')}
                className="ac-composer__send"
                onClick={() => void commitEdit()}
                type="button"
              >
                <Icon name="check" size={17} />
              </button>
            ) : draft.trim() ? (
              <button aria-label={text('send', 'Send')} className="ac-composer__send" onClick={() => void send()} type="button">
                <Icon name="send" size={17} />
              </button>
            ) : (
              <Tooltip content={text('recordVoice', 'Record a voice note')} delay={200}>
                <button
                  aria-label={text('recordVoice', 'Record a voice note')}
                  className="ac-composer__send ac-composer__send--rec"
                  onClick={() => void startRecording()}
                  type="button"
                >
                  <Icon name="mic" size={17} />
                </button>
              </Tooltip>
            )}
          </div>
        )}

        {attachOpen ? (
          <div className="ac-composer__attach" role="menu">
            <button
              className="ac-composer__attach-item"
              onClick={() => {
                setAttachOpen(false);
                pictureInput.current?.click();
              }}
              role="menuitem"
              type="button"
            >
              <Icon name="image" size={16} />
              {text('attachPhoto', 'Photo or video')}
            </button>
            <button
              className="ac-composer__attach-item"
              onClick={() => {
                setAttachOpen(false);
                fileInput.current?.click();
              }}
              role="menuitem"
              type="button"
            >
              <Icon name="file" size={16} />
              {text('attachFile', 'File or document')}
            </button>
            <button className="ac-composer__attach-item" onClick={() => void startRecording()} role="menuitem" type="button">
              <Icon name="mic" size={16} />
              {text('attachVoice', 'Voice note')}
            </button>
          </div>
        ) : null}

        {emojiOpen ? (
          <div aria-label={text('emoji', 'Emoji')} className="ac-composer__emoji" role="listbox">
            {EMOJI.map(emoji => (
              <button
                key={emoji}
                onClick={() => {
                  if (editing) setEditing({ ...editing, text: `${editing.text}${emoji}` });
                  else setDraft(current => `${current}${emoji}`);
                  inputRef.current?.focus();
                }}
                type="button"
              >
                {emoji}
              </button>
            ))}
          </div>
        ) : null}

        <div className="ac-composer__meta">
          <span className="ac-composer__hint">
            <Icon name={settings.lanEncryption ? 'lock' : 'shield'} size={12} />
            {settings.lanEncryption
              ? text('encryptionOn', 'End-to-end encrypted on this local network')
              : text('encryptionOff', 'Not encrypted — set the network passphrase')}
          </span>
          <span>{text('shortcutSend', 'Enter to send · Shift+Enter for a new line')}</span>
        </div>

        <input
          accept="image/*,video/*"
          hidden
          multiple
          onChange={event => {
            void pickFile(event.target.files?.[0]);
            event.target.value = '';
          }}
          ref={pictureInput}
          type="file"
        />
        <input
          accept="*/*"
          hidden
          onChange={event => {
            void pickFile(event.target.files?.[0]);
            event.target.value = '';
          }}
          ref={fileInput}
          type="file"
        />
      </footer>

      <ContextMenu
        anchor={menu ? { x: menu.x, y: menu.y } : null}
        items={menu ? menuFor(menu.message) : []}
        label={text('messageMenu', 'Message actions')}
        onClose={() => setMenu(null)}
        onReaction={emoji => {
          if (menu) void react(menu.message, emoji);
        }}
        open={Boolean(menu)}
        reactions={QUICK_REACTIONS}
      />

      {confirm ? (
        <div aria-label={text('delete', 'Delete')} aria-modal="true" className="ac-chat-confirm" role="alertdialog">
          <div className="ac-chat-confirm__card">
            <span className="ac-chat-confirm__icon">
              <Icon name="alertCircle" size={20} />
            </span>
            <h3>
              {confirm.everyone
                ? text('deleteForEveryone', 'Delete for everyone')
                : text('deleteForMe', 'Delete for me')}
            </h3>
            <p>
              {confirm.everyone
                ? text('confirmDeleteEveryone', 'Everyone loses this message. It cannot be undone.')
                : text('confirmDeleteMine', 'This message disappears from this device only.')}
            </p>
            <div className="ac-chat-confirm__actions">
              <button className="ac-btn ac-btn--secondary" onClick={() => setConfirm(null)} type="button">
                {text('cancel', 'Cancel')}
              </button>
              <button
                className="ac-btn ac-btn--danger"
                onClick={() => {
                  const target = confirm;
                  setConfirm(null);
                  void remove(selectionOn && target.message.id === selected[0] ? selected : [target.message.id], target.everyone);
                }}
                type="button"
              >
                {text('delete', 'Delete')}
              </button>
            </div>
          </div>
        </div>
      ) : null}

      {forwarding ? (
        <ForwardPicker
          current={chatId}
          onCancel={() => setForwarding(false)}
          onPick={ids => void forwardTo(ids)}
          peers={peers}
        />
      ) : null}
    </section>
  );
}

/* ===== One virtualized row =============================================== */

type RowProps = {
  index: number;
  style: CSSProperties;
  wrapRef: (element: Element | null) => void;
  message: ChatMessage;
  attachment: Attachment | undefined;
  reactions: ReactionTally[];
  mine: boolean;
  /** Consecutive message from the same sender: the avatar slot collapses. */
  grouped: boolean;
  /** Names for the ids storage leaves in `sender` and in a reaction tally. */
  nameOf: (id: string) => string;
  /** Only while multi-select is on; otherwise the row shows no checkbox. */
  selected?: boolean;
  starred: boolean;
  byId: Map<string, ChatMessage>;
  onSelect?: (on: boolean) => void;
  onReply: () => void;
  onReact: (emoji: string) => void;
  onContextMenu: (point: { x: number; y: number }) => void;
  onTransfer: (id: string, action: 'pause' | 'resume' | 'retry' | 'cancel') => void;
  onOpen: (id: string) => void;
  onVoiceSource: (id: string) => Promise<string | null>;
  onVoiceError: (error: unknown) => void;
  /** Reads one finished attachment's bytes, for the picture this row may draw. */
  previewSource: AttachmentPreviewSource;
  /** The Settings → Calls output, honoured where the engine can name one. */
  speaker?: string;
};

function MessageRow({
  index,
  style,
  wrapRef,
  message,
  attachment,
  reactions,
  mine,
  grouped,
  nameOf,
  selected,
  starred,
  byId,
  onSelect,
  onReply,
  onReact,
  onContextMenu,
  onTransfer,
  onOpen,
  onVoiceSource,
  onVoiceError,
  previewSource,
  speaker,
}: RowProps) {
  // Pictures come from a shared cache, so this row and the media grid agree on what a
  // finished attachment looks like and only one of them pays for the bytes.
  const previewVersion = useSyncExternalStore(
    subscribeAttachmentPreviews,
    attachmentPreviewVersion,
    attachmentPreviewVersion,
  );
  useEffect(() => {
    if (attachment) loadAttachmentPreview(previewSource, attachment);
  }, [attachment, previewSource, previewVersion]);
  const preview = attachmentPreview(attachment?.id);
  const variant = variantOf(message, preview, attachment);
  const [playing, setPlaying] = useState(false);
  const [playedSeconds, setPlayedSeconds] = useState(0);
  const [measuredSeconds, setMeasuredSeconds] = useState<number | null>(null);
  const audioRef = useRef<HTMLAudioElement | null>(null);

  const attachmentId = attachment?.id;
  // Callbacks stay in refs so a parent re-render cannot tear down a playing note.
  const sourceRef = useRef(onVoiceSource);
  sourceRef.current = onVoiceSource;
  const errorRef = useRef(onVoiceError);
  errorRef.current = onVoiceError;
  const speakerRef = useRef(speaker);
  speakerRef.current = speaker;

  // Loading the bytes on mount keeps the press-to-play path synchronous: WebKit only
  // starts media from the gesture itself, not from a promise resolved later.
  useEffect(() => {
    if (variant !== 'voice' || !attachmentId) return;
    let live = true;
    sourceRef
      .current(attachmentId)
      .then(source => {
        if (!live) return;
        if (!source) {
          errorRef.current(new Error('The recording is not on this device yet.'));
          return;
        }
        const audio = new Audio(source);
        audioRef.current = audio;
        applySinkId(audio, speakerRef.current);
        audio.onplay = () => setPlaying(true);
        audio.onpause = () => setPlaying(false);
        audio.onended = () => {
          setPlaying(false);
          setPlayedSeconds(0);
        };
        audio.ontimeupdate = () => setPlayedSeconds(audio.currentTime);
        audio.onerror = () => errorRef.current(new Error('This recording could not be played.'));
        audio.onloadedmetadata = () => {
          if (Number.isFinite(audio.duration) && audio.duration > 0) setMeasuredSeconds(Math.round(audio.duration));
        };
      })
      .catch(error => errorRef.current(error));
    return () => {
      live = false;
      audioRef.current?.pause();
      audioRef.current = null;
    };
  }, [attachmentId, variant]);

  // Settings is another window, so the chosen output can move while a note is still playing.
  useEffect(() => {
    if (audioRef.current) applySinkId(audioRef.current, speaker);
  }, [speaker]);

  const toggleVoice = useCallback(() => {
    const audio = audioRef.current;
    if (!audio) {
      errorRef.current(new Error('The recording is still loading.'));
      return;
    }
    if (audio.paused) void audio.play().catch(error => errorRef.current(error));
    else audio.pause();
  }, []);
  const inputs: ReactionInput[] = reactions.flatMap(tally =>
    tally.peers.map(peerId => ({ emoji: tally.emoji, by: nameOf(peerId) })),
  );
  const quoted = message.replyTo ? byId.get(message.replyTo) : undefined;
  const wrap = { 'data-index': index, ref: wrapRef, style };

  if (message.kind === 'event') {
    return (
      <div {...wrap}>
        <span className="ac-convo__event">{message.text}</span>
      </div>
    );
  }

  return (
    <div {...wrap} className={cx('ac-convo__row', mine && 'ac-convo__row--mine')}>
      <span className="ac-convo__stack">
        <MessageBubble
          actions={
            <>
              <IconButton iconSize={14} label={text('reply', 'Reply')} name="reply" onClick={onReply} />
              <IconButton
                iconSize={14}
                label={text('react', 'React')}
                name="smilePlus"
                onClick={() => onReact('👍')}
              />
            </>
          }
          delivery={mine ? deliveryOf(message) : undefined}
          edited={message.edited}
          editedLabel={text('editedTag', '(edited)')}
          fileName={attachment?.name ?? message.fileName}
          fileSize={attachment?.bytes}
          forwarded={message.forwarded}
          forwardedLabel={text('forwardedLabel', 'Forwarded')}
          media={
            preview && variant === 'image' && attachment
              ? [{ alt: attachment.name, src: preview }]
              : undefined
          }
          grouped={grouped}
          mine={mine}
          onContextMenu={onContextMenu}
          onDownload={attachment ? () => onOpen(attachment.id) : undefined}
          onCancel={attachment ? () => void onTransfer(attachment.id, 'cancel') : undefined}
          onOpenMedia={attachment ? () => onOpen(attachment.id) : undefined}
          onPause={attachment ? () => void onTransfer(attachment.id, 'pause') : undefined}
          onReaction={onReact}
          onRetry={attachment ? () => void onTransfer(attachment.id, 'retry') : undefined}
          onResume={attachment ? () => void onTransfer(attachment.id, 'resume') : undefined}
          onSelectedChange={onSelect}
          onTogglePlay={variant === 'voice' ? () => void toggleVoice() : undefined}
          playedSeconds={playing ? Math.floor(playedSeconds) : 0}
          playing={playing}
          reactions={inputs}
          reply={quoted ? { sender: nameOf(quoted.sender), text: quoted.text } : undefined}
          selected={selected}
          sender={nameOf(message.sender)}
          senderColor={avatarPaletteFor(message.sender)}
          text={variant === 'deleted' ? text('deletedMessage', 'This message was deleted.') : message.text}
          time={messageTime(message)}
          totalSize={attachment?.bytes}
          transfer={transferOf(attachment)}
          transferred={
            attachment?.progress === null || !attachment
              ? undefined
              : Math.round(attachment.progress * attachment.bytes)
          }
          variant={variant}
          voiceSeconds={attachment?.durationMs ? Math.round(attachment.durationMs / 1000) : measuredSeconds ?? undefined}
          waveformSeed={attachment?.id ?? message.id}
        />
        {starred ? (
          <span className="ac-convo__flag">
            <Icon name="star" size={11} />
            {text('starredTab', 'Starred')}
          </span>
        ) : null}
      </span>
    </div>
  );
}

/* ===== Forward target picker ============================================= */

function ForwardPicker({
  peers,
  current,
  onPick,
  onCancel,
}: {
  peers: Peer[];
  current: string;
  onPick: (chatIds: string[]) => void;
  onCancel: () => void;
}) {
  const [picked, setPicked] = useState<string[]>([]);
  return (
    <div
      aria-label={text('forwardPick', 'Choose where to forward')}
      aria-modal="true"
      className="ac-chat-confirm"
      role="dialog"
    >
      <div className="ac-chat-confirm__card ac-chat-confirm__card--wide">
        <h3>{text('forwardPick', 'Choose where to forward')}</h3>
        <div className="ac-chat-confirm__list">
          {peers
            .filter(entry => entry.id !== current)
            .map(entry => (
              <button
                className={cx('ac-chat-row', picked.includes(entry.id) && 'ac-chat-row--on')}
                key={entry.id}
                onClick={() =>
                  setPicked(current2 =>
                    current2.includes(entry.id) ? current2.filter(id => id !== entry.id) : [...current2, entry.id],
                  )
                }
                type="button"
              >
                <Avatar
                  group={entry.group}
                  initials={entry.initials || initialsFor(entry.name)}
                  name={entry.name}
                  size={24}
                />
                <span className="ac-chat-row__preview">{entry.name}</span>
                {picked.includes(entry.id) ? <Icon name="check" size={15} /> : null}
              </button>
            ))}
        </div>
        <div className="ac-chat-confirm__actions">
          <button className="ac-btn ac-btn--secondary" onClick={onCancel} type="button">
            {text('cancel', 'Cancel')}
          </button>
          <button
            className="ac-btn ac-btn--primary"
            disabled={picked.length === 0}
            onClick={() => onPick(picked)}
            type="button"
          >
            {text('forwardSend', 'Forward')}
          </button>
        </div>
      </div>
    </div>
  );
}
