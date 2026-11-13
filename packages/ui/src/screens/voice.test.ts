import { describe, expect, it } from 'vitest';
import { VOICE_SAMPLE_RATE, downsample, mergeSamples, wavBlob } from './voice';

function ascii(bytes: Uint8Array, offset: number, length: number): string {
  return String.fromCharCode(...bytes.slice(offset, offset + length));
}

describe('voice note encoding', () => {
  it('joins capture blocks in order', () => {
    const merged = mergeSamples([new Float32Array([1, 2]), new Float32Array([3]), new Float32Array([4, 5])]);
    expect(Array.from(merged)).toEqual([1, 2, 3, 4, 5]);
    expect(mergeSamples([])).toHaveLength(0);
  });

  it('averages source windows when it decimates', () => {
    const input = new Float32Array(48_000).fill(0.5);
    const output = downsample(input, 48_000, VOICE_SAMPLE_RATE);
    expect(output).toHaveLength(16_000);
    expect(output[0]).toBeCloseTo(0.5);
    expect(downsample(input, VOICE_SAMPLE_RATE, VOICE_SAMPLE_RATE)).toBe(input);
  });

  it('writes a mono 16-bit WAV header around the samples', async () => {
    const blob = wavBlob(new Float32Array([0.5, -0.5]), VOICE_SAMPLE_RATE);
    expect(blob.type).toBe('audio/wav');
    const bytes = new Uint8Array(await blob.arrayBuffer());
    const view = new DataView(bytes.buffer);
    expect(bytes).toHaveLength(44 + 4);
    expect(ascii(bytes, 0, 4)).toBe('RIFF');
    expect(ascii(bytes, 8, 4)).toBe('WAVE');
    expect(ascii(bytes, 36, 4)).toBe('data');
    expect(view.getUint32(4, true)).toBe(40);
    expect(view.getUint16(20, true)).toBe(1);
    expect(view.getUint16(22, true)).toBe(1);
    expect(view.getUint32(24, true)).toBe(VOICE_SAMPLE_RATE);
    expect(view.getUint16(34, true)).toBe(16);
    expect(view.getUint32(40, true)).toBe(4);
    expect(view.getInt16(44, true)).toBe(Math.trunc(0.5 * 0x7fff));
    expect(view.getInt16(46, true)).toBe(-(0.5 * 0x8000));
  });
});
