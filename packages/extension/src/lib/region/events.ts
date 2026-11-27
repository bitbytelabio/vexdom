export const REGION_CHANNEL = 'vexdom/region';

export type RegionCommand =
  { channel: typeof REGION_CHANNEL; op: 'pick' } | { channel: typeof REGION_CHANNEL; op: 'cancel' };

/** The box the user drew, in CSS pixels of the viewport it was drawn on. */
export interface RegionPick {
  rect: { x: number; y: number; width: number; height: number };
  dpr: number;
  viewport: { width: number; height: number };
}

export function isRegionCommand(message: unknown): message is RegionCommand {
  return typeof message === 'object' && message !== null && (message as RegionCommand).channel === REGION_CHANNEL;
}
