import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  attachmentPreview,
  attachmentPreviewVersion,
  clearAttachmentPreviews,
  loadAttachmentPreview,
  subscribeAttachmentPreviews,
  type AttachmentPreviewSource,
  type PreviewRequest,
} from './attachmentPreviews';

const PNG = 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==';

const flush = () => new Promise<void>(resolve => setTimeout(resolve, 0));

/** A finished picture, unless the test says otherwise. */
const row = (id: string, over: Partial<PreviewRequest> = {}): PreviewRequest => ({
  id,
  kind: 'image',
  state: 'done',
  ...over,
});

/** A bridge whose attachment bytes are scripted per id, counting how often each is asked. */
function scripted(
  answers: Record<string, string | null | Error>,
): AttachmentPreviewSource & { asks: Record<string, number> } {
  const asks: Record<string, number> = {};
  return {
    asks,
    attachmentData(attachmentId) {
      asks[attachmentId] = (asks[attachmentId] ?? 0) + 1;
      const answer = answers[attachmentId];
      return answer instanceof Error ? Promise.reject(answer) : Promise.resolve(answer ?? null);
    },
  };
}

beforeEach(() => {
  clearAttachmentPreviews();
});

describe('attachment preview cache', () => {
  it('asks once per attachment however many surfaces draw it', async () => {
    const source = scripted({ a7: PNG });
    loadAttachmentPreview(source, row('a7'));
    loadAttachmentPreview(source, row('a7'));
    loadAttachmentPreview(source, row('a7'));
    await flush();
    expect(source.asks.a7).toBe(1);
    expect(attachmentPreview('a7')).toBe(PNG);
  });

  it('asks for nothing that cannot answer with a picture', async () => {
    const source = scripted({});
    for (const attachment of [
      row('v1', { kind: 'video' }),
      row('n1', { kind: 'audio' }),
      row('f1', { kind: 'file' }),
      row('a8', { state: 'uploading' }),
      row('a9', { state: 'downloading' }),
      row('a4', { state: 'failed' }),
      row('a2', { state: 'paused' }),
    ]) {
      loadAttachmentPreview(source, attachment);
    }
    await flush();
    expect(source.asks).toEqual({});
  });

  it('keeps a refusal as no picture instead of reading the file again', async () => {
    const source = scripted({ big: new Error('too large to inline') });
    loadAttachmentPreview(source, row('big'));
    await flush();
    expect(attachmentPreview('big')).toBeNull();
    loadAttachmentPreview(source, row('big'));
    await flush();
    expect(source.asks.big).toBe(1);
  });

  it('draws a file chip for an id with no bytes behind it', async () => {
    const source = scripted({ gone: null });
    expect(attachmentPreview('gone')).toBeUndefined();
    loadAttachmentPreview(source, row('gone'));
    await flush();
    expect(attachmentPreview('gone')).toBeNull();
  });

  it('tells every subscriber, and only once, per change', async () => {
    const heard = vi.fn();
    const stop = subscribeAttachmentPreviews(heard);
    const before = attachmentPreviewVersion();
    loadAttachmentPreview(scripted({ a7: PNG }), row('a7'));
    await flush();
    stop();
    loadAttachmentPreview(scripted({ a8: PNG }), row('a8'));
    await flush();
    expect(heard).toHaveBeenCalledTimes(1);
    expect(attachmentPreviewVersion()).toBe(before + 2);
  });

  it('forgets the thread that closed, so the next one asks again', async () => {
    const source = scripted({ a7: PNG });
    loadAttachmentPreview(source, row('a7'));
    await flush();
    clearAttachmentPreviews();
    expect(attachmentPreview('a7')).toBeUndefined();
    loadAttachmentPreview(source, row('a7'));
    await flush();
    expect(source.asks.a7).toBe(2);
    expect(attachmentPreview('a7')).toBe(PNG);
  });

  it('names an unknown attachment as nothing to draw', () => {
    expect(attachmentPreview(undefined)).toBeNull();
    expect(attachmentPreview('')).toBeNull();
    expect(attachmentPreview('never-asked')).toBeUndefined();
  });
});
