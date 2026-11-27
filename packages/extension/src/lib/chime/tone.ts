export type ChimeKind = 'done' | 'failed' | 'attention';

/** Asks an open offscreen page to play a chime: `{ channel, kind }`. */
export const CHIME_CHANNEL = 'vexdom/chime';

export const isChimeKind = (value: unknown): value is ChimeKind =>
  value === 'done' || value === 'failed' || value === 'attention';

interface Note {
  hz: number;
  ms: number;
  /** Silence before the next note. */
  gapMs?: number;
}

const TUNES: Record<ChimeKind, Note[]> = {
  attention: [
    { hz: 880, ms: 120, gapMs: 40 },
    { hz: 660, ms: 120 },
  ],
  done: [
    { hz: 523, ms: 110 },
    { hz: 784, ms: 110 },
  ],
  failed: [{ hz: 330, ms: 260 }],
};

const PEAK = 0.15;
const ATTACK_S = 0.01;

/** A short sine-wave chime, one oscillator per note; resolves once the last note has faded. */
export async function playTone(kind: ChimeKind): Promise<void> {
  const context = new AudioContext();
  try {
    let at = context.currentTime;
    for (const { hz, ms, gapMs = 0 } of TUNES[kind]) {
      const oscillator = context.createOscillator();
      const gain = context.createGain();
      oscillator.type = 'sine';
      oscillator.frequency.value = hz;
      gain.gain.setValueAtTime(0, at);
      gain.gain.linearRampToValueAtTime(PEAK, at + ATTACK_S);
      gain.gain.linearRampToValueAtTime(0, at + ms / 1000);
      oscillator.connect(gain).connect(context.destination);
      oscillator.start(at);
      oscillator.stop(at + ms / 1000);
      at += (ms + gapMs) / 1000;
    }
    await new Promise((resolve) => setTimeout(resolve, (at - context.currentTime) * 1000));
  } finally {
    await context.close().catch(() => undefined);
  }
}
