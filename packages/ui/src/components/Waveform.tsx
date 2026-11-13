import { useMemo, type CSSProperties } from 'react';
import { Icon, cx } from './Icon';
import { clampPercent, waveformBars, waveformProgress } from './logic';

export type WaveformProps = {
  /**
   * Explicit amplitudes (0..1). When omitted they are generated deterministically
   * from `seed`, so the same voice note always draws the same shape.
   */
  bars?: readonly number[];
  seed?: string | number;
  count?: number;
  /** 0..1 played fraction, or pass positionSeconds + durationSeconds instead. */
  progress?: number;
  positionSeconds?: number;
  durationSeconds?: number;
  /** Adds the travelling scaleY animation used while audio is playing. */
  playing?: boolean;
  /** 52px call-poster height instead of the 32px voice-note height. */
  tall?: boolean;
  /** Makes the waveform clickable / arrow-keyable to seek. */
  onSeek?: (fraction: number) => void;
  /** Accessible name, e.g. "Voice note waveform". */
  label?: string;
  className?: string;
};

function barStyle(amplitude: number): CSSProperties {
  return { height: `${clampPercent(amplitude * 100)}%` };
}

/**
 * Voice-note / call waveform. Bars before the play head use the accent gradient,
 * the rest stay muted, matching the voice-note bubbles on the showcase board.
 */
export function Waveform({
  bars,
  seed = 'voice',
  count = 32,
  progress,
  positionSeconds,
  durationSeconds,
  playing = false,
  tall = false,
  onSeek,
  label = 'Voice note waveform',
  className,
}: WaveformProps) {
  const amplitudes = useMemo(
    () => (bars ? Array.from(bars) : waveformBars(seed, count)),
    [bars, seed, count],
  );
  const played =
    progress ??
    (typeof positionSeconds === 'number' && typeof durationSeconds === 'number'
      ? waveformProgress(positionSeconds, durationSeconds)
      : 0);
  const playedBars = Math.round(played * amplitudes.length);

  const lines = amplitudes.map((amplitude, index) => (
    <i
      className={cx('ac-waveform__bar', index < playedBars && 'ac-waveform__bar--played')}
      key={index}
      style={barStyle(amplitude)}
    />
  ));

  const shellClass = cx(
    'ac-waveform',
    tall && 'ac-waveform--tall',
    playing && 'ac-waveform--playing',
    onSeek && 'ac-waveform--seekable',
    className,
  );

  if (!onSeek) {
    return (
      <span aria-label={label} className={shellClass} role="img">
        {lines}
      </span>
    );
  }

  const seekBy = (delta: number) => onSeek(Math.min(1, Math.max(0, played + delta)));

  return (
    <span
      aria-label={label}
      aria-valuemax={100}
      aria-valuemin={0}
      aria-valuenow={Math.round(played * 100)}
      className={shellClass}
      onClick={(event) => {
        const rect = event.currentTarget.getBoundingClientRect();
        if (rect.width === 0) return;
        onSeek(Math.min(1, Math.max(0, (event.clientX - rect.left) / rect.width)));
      }}
      onKeyDown={(event) => {
        if (event.key === 'ArrowRight') {
          event.preventDefault();
          seekBy(0.05);
        } else if (event.key === 'ArrowLeft') {
          event.preventDefault();
          seekBy(-0.05);
        }
      }}
      role="slider"
      tabIndex={0}
    >
      {lines}
    </span>
  );
}

/** Play / pause affordance that sits in front of a waveform in a voice note. */
export function WaveformPlayButton({
  playing,
  onClick,
  label = playing ? 'Pause voice note' : 'Play voice note',
}: {
  playing: boolean;
  onClick: () => void;
  label?: string;
}) {
  return (
    <button aria-label={label} className="ac-bubble__voice-btn" onClick={onClick} type="button">
      <Icon name={playing ? 'pause' : 'play'} size={14} />
    </button>
  );
}
