export const INDICATOR_CHANNEL = 'vexdom/indicator';

/** Matches --brand in assets/globals.css; the background has no stylesheet to read it from. */
export const INDICATOR_COLOR = '#ff7a3d';

/** The frame's colour while a run waits for the user. */
export const WAITING_COLOR = '#f5c542';

export type IndicatorCommand =
  /** A run works here: `glow` frames the page, `waiting` says what it waits on, a hand-off being the user's turn on the page. */
  | { channel: typeof INDICATOR_CHANNEL; op: 'busy'; glow: boolean; waiting?: { text: string; handOff: boolean } }
  | { channel: typeof INDICATOR_CHANNEL; op: 'idle' }
  /** Out of the way for a screenshot, then back. */
  | { channel: typeof INDICATOR_CHANNEL; op: 'hush' }
  | { channel: typeof INDICATOR_CHANNEL; op: 'unhush' };

export function isIndicatorCommand(message: unknown): message is IndicatorCommand {
  return typeof message === 'object' && message !== null && (message as IndicatorCommand).channel === INDICATOR_CHANNEL;
}
