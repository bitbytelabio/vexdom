import { browser } from 'wxt/browser';
import { announceAll, elementForProbe, FRAME_PROBE_KEY } from './probe';
import { FRAME_MARK_ATTRIBUTE, isFrameProbe, type FrameDescription, type FrameOrigin } from './events';
import { clearRefLabels, drawRefLabels } from './ref-labels';

/** The nonce this frame's parent last posted into it (`announceFrame`). */
let announced: string | null = null;

export function exposeFrameProbe(): void {
  window.addEventListener('message', (event) => {
    const nonce = (event.data as Record<string, unknown> | null)?.[FRAME_PROBE_KEY];
    if (event.source === window.parent && window.parent !== window && typeof nonce === 'string') announced = nonce;
  });
  browser.runtime.onMessage.addListener((message: unknown, _sender, sendResponse) => {
    if (!isFrameProbe(message)) return;
    switch (message.op) {
      case 'describe':
        sendResponse(describeFrame());
        return;
      case 'mark':
        document.documentElement.setAttribute(FRAME_MARK_ATTRIBUTE, message.token);
        sendResponse({ ok: true });
        return;
      case 'unmark':
        document.documentElement.removeAttribute(FRAME_MARK_ATTRIBUTE);
        sendResponse({ ok: true });
        return;
      case 'locate':
        sendResponse(locateChild(message.probe));
        return;
      case 'announceAll':
        announceAll();
        sendResponse({ ok: true });
        return;
      case 'labels':
        void drawRefLabels(message.labels).then((drawn) => sendResponse({ ok: true, drawn }));
        return true;
      case 'unlabel':
        clearRefLabels();
        sendResponse({ ok: true });
        return;
      case 'claim':
        sendResponse({ nonce: announced });
        return;
    }
  });
}

function describeFrame(): FrameDescription {
  return {
    url: location.href,
    title: document.title,
    top: window === window.top,
    viewport: { w: window.innerWidth, h: window.innerHeight },
  };
}

function locateChild(probe: string): FrameOrigin | null {
  const child = elementForProbe(probe);
  if (!child) return null;
  const rect = child.getBoundingClientRect();
  const style = getComputedStyle(child);
  return {
    x: rect.x + child.clientLeft + parseFloat(style.paddingLeft),
    y: rect.y + child.clientTop + parseFloat(style.paddingTop),
  };
}
