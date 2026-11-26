import type { RefLabel } from './ref-labels';

export const FRAME_CHANNEL = 'vexdom/frame';

export const FRAME_MARK_ATTRIBUTE = 'data-vexdom-frame';

export type FrameProbeBody =
  | { op: 'describe' }
  | { op: 'mark'; token: string }
  | { op: 'unmark' }
  | { op: 'locate'; probe: string }
  | { op: 'announceAll' }
  | { op: 'labels'; labels: RefLabel[] }
  | { op: 'unlabel' }
  | { op: 'claim' };

export type FrameProbe = FrameProbeBody & { channel: typeof FRAME_CHANNEL };

export interface FrameDescription {
  url: string;
  title: string;
  top: boolean;
  viewport: { w: number; h: number };
}

export interface FrameOrigin {
  x: number;
  y: number;
}

export function isFrameProbe(message: unknown): message is FrameProbe {
  return (
    typeof message === 'object' &&
    message !== null &&
    (message as { channel?: unknown }).channel === FRAME_CHANNEL &&
    typeof (message as { op?: unknown }).op === 'string'
  );
}
