/** Voice notes travel as 16 kHz mono WAV: small on a LAN and playable in every webview. */
export const VOICE_SAMPLE_RATE = 16_000;

/** Joins the per-buffer PCM blocks a capture node delivers into one contiguous take. */
export function mergeSamples(chunks: Float32Array[]): Float32Array {
  const total = chunks.reduce((sum, chunk) => sum + chunk.length, 0);
  const merged = new Float32Array(total);
  let offset = 0;
  for (const chunk of chunks) {
    merged.set(chunk, offset);
    offset += chunk.length;
  }
  return merged;
}

/** Averages every source-rate window, which is enough fidelity for speech. */
export function downsample(input: Float32Array, from: number, to: number): Float32Array {
  if (to >= from) return input;
  const ratio = from / to;
  const output = new Float32Array(Math.round(input.length / ratio));
  for (let index = 0; index < output.length; index += 1) {
    const start = Math.round(index * ratio);
    const end = Math.min(input.length, Math.round((index + 1) * ratio));
    let sum = 0;
    for (let cursor = start; cursor < end; cursor += 1) sum += input[cursor];
    output[index] = end > start ? sum / (end - start) : 0;
  }
  return output;
}

/** A canonical 44-byte PCM WAV header in front of 16-bit little-endian samples. */
export function wavBlob(samples: Float32Array, sampleRate: number): Blob {
  const buffer = new ArrayBuffer(44 + samples.length * 2);
  const view = new DataView(buffer);
  const text = (offset: number, value: string) => {
    for (let index = 0; index < value.length; index += 1) view.setUint8(offset + index, value.charCodeAt(index));
  };
  text(0, 'RIFF');
  view.setUint32(4, 36 + samples.length * 2, true);
  text(8, 'WAVE');
  text(12, 'fmt ');
  view.setUint32(16, 16, true);
  view.setUint16(20, 1, true);
  view.setUint16(22, 1, true);
  view.setUint32(24, sampleRate, true);
  view.setUint32(28, sampleRate * 2, true);
  view.setUint16(32, 2, true);
  view.setUint16(34, 16, true);
  text(36, 'data');
  view.setUint32(40, samples.length * 2, true);
  let offset = 44;
  for (const sample of samples) {
    const clamped = Math.max(-1, Math.min(1, sample));
    view.setInt16(offset, clamped < 0 ? clamped * 0x8000 : clamped * 0x7fff, true);
    offset += 2;
  }
  return new Blob([buffer], { type: 'audio/wav' });
}
