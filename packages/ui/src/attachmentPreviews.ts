/* ------------------------------------------------------------------ *
 * Attachment pictures, pulled one at a time and shared by the shell.
 *
 * A transfer row says the bytes are on this machine; it does not carry them.
 * Putting a picture in every row would send the whole chat folder across the
 * command seam each time the thread reloads, so a surface that is about to
 * draw one attachment asks for that one, and every surface naming the same id
 * shows the same answer.
 * ------------------------------------------------------------------ */

import { useEffect, useSyncExternalStore } from 'react';

/** The part of a transfer row this cache reads; keeps the module free of the bridge. */
export type PreviewRequest = { id: string; kind: string; state: string };

/** The part of the bridge that can read one attachment's bytes. */
export type AttachmentPreviewSource = {
  attachmentData(attachmentId: string): Promise<string | null>;
};

/** `undefined` = never asked, `null` = asked and no picture can be drawn from it. */
const urls = new Map<string, string | null>();
const asked = new Set<string>();
let version = 0;
const listeners = new Set<() => void>();

function notify(): void {
  version += 1;
  for (const listener of listeners) listener();
}

export function subscribeAttachmentPreviews(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** Stable between changes, so React can use it as an external-store snapshot. */
export function attachmentPreviewVersion(): number {
  return version;
}

export function attachmentPreview(attachmentId: string | undefined | null): string | null | undefined {
  if (!attachmentId) return null;
  return urls.get(attachmentId);
}

/**
 * Whether this row is one a picture can be drawn from: the bytes have to be here, and
 * only a still image has a rendering that needs no decoding — a video's first frame is
 * not something either webview will hand us from a file path.
 */
function previewable(attachment: PreviewRequest): boolean {
  return attachment.kind === 'image' && attachment.state === 'done';
}

/**
 * Reads one attachment's bytes, once. An answer that fails is kept as "no picture"
 * rather than retried: this only asks for rows already marked complete, so a refusal
 * is the file being too large to inline or gone from the disk, neither of which a
 * second ask fixes.
 */
export function loadAttachmentPreview(source: AttachmentPreviewSource, attachment: PreviewRequest): void {
  if (!previewable(attachment) || asked.has(attachment.id)) return;
  asked.add(attachment.id);
  source.attachmentData(attachment.id).then(
    (dataUrl) => {
      urls.set(attachment.id, dataUrl ?? null);
      notify();
    },
    () => {
      urls.set(attachment.id, null);
      notify();
    },
  );
}

/**
 * A different thread is open. Forget what we hold, so the pictures of the chat that
 * closed do not sit in memory beside the ones being drawn now.
 */
export function clearAttachmentPreviews(): void {
  if (urls.size === 0 && asked.size === 0) return;
  urls.clear();
  asked.clear();
  notify();
}

/**
 * A list's view of the cache: it repaints when a picture arrives, asks for every row it
 * was handed that can be drawn, and hands back the lookup to render from plus the version
 * that changed with it — a caller that memoises a derived list needs that number in its
 * dependencies, or the memo keeps the rows built before the bytes arrived.
 */
export function useAttachmentPreviews(
  source: AttachmentPreviewSource,
  attachments: readonly PreviewRequest[],
): [read: (attachmentId: string) => string | null | undefined, version: number] {
  const version = useSyncExternalStore(
    subscribeAttachmentPreviews,
    attachmentPreviewVersion,
    attachmentPreviewVersion,
  );
  useEffect(() => {
    for (const attachment of attachments) loadAttachmentPreview(source, attachment);
  }, [attachments, source, version]);
  return [attachmentPreview, version];
}