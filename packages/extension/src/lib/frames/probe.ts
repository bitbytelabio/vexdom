/**
 * Naming a child frame without `runtime.getFrameId` (Chrome for Testing has none, and the answer
 * differs between browsers): the parent posts a fresh nonce into the frame, the frame's own
 * content script keeps it (`frames/host.ts`), and the background asks each frame for its nonce
 * (`bridge/snapshot-frames.ts` `claimFrame`). The parent remembers which element it posted each
 * nonce into, so the background can later ask where that frame sits.
 */

/** What a parent posts into a child frame so the background can tell which frame it is. */
export const FRAME_PROBE_KEY = 'vexdomFrame';

const announcedTo = new Map<string, WeakRef<Element>>();

export function mintKey(): string {
  if (typeof crypto.randomUUID === 'function') return crypto.randomUUID();
  // randomUUID needs a secure context, which a content script on an http: page is not.
  const bytes = crypto.getRandomValues(new Uint8Array(16));
  bytes[6] = (bytes[6] & 0x0f) | 0x40;
  bytes[8] = (bytes[8] & 0x3f) | 0x80;
  const hex = Array.from(bytes, (byte) => byte.toString(16).padStart(2, '0')).join('');
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}

/** Posts a fresh nonce into a frame element's window; undefined when it has none to post to. */
export function announceFrame(element: Element): string | undefined {
  const target = (element as HTMLIFrameElement).contentWindow;
  if (!target) return undefined;
  const nonce = mintKey();
  try {
    target.postMessage({ [FRAME_PROBE_KEY]: nonce }, '*');
  } catch {
    return undefined;
  }
  announcedTo.set(nonce, new WeakRef(element));
  return nonce;
}

/** The frame element a nonce was posted into, while it is still in the document. */
export function elementForProbe(nonce: string): Element | null {
  const element = announcedTo.get(nonce)?.deref();
  return element?.isConnected ? element : null;
}

/** Announces into every frame of this document, so each can be claimed and then located. */
export function announceAll(): void {
  for (const element of document.querySelectorAll('iframe,frame')) announceFrame(element);
}
