import { browser } from 'wxt/browser';
import { BRIDGE_CHANNEL } from '@/lib/actions/protocol';
import { OVERLAY_ATTRIBUTE } from '@/lib/overlay';
import { INDICATOR_COLOR, WAITING_COLOR, type IndicatorCommand } from './events';

type Busy = Extract<IndicatorCommand, { op: 'busy' }>;

const HOST_ID = 'vexdom-run-glow';

const STYLE = `
  :host { all: initial; }
  .frame {
    position: fixed; inset: 0; pointer-events: none;
    box-shadow: inset 0 0 0 2px var(--glow), inset 0 0 28px 4px color-mix(in srgb, var(--glow) 40%, transparent);
    animation: pulse 2s ease-in-out infinite alternate;
  }
  @keyframes pulse { from { opacity: 0.55; } to { opacity: 1; } }
  @media (prefers-reduced-motion: reduce) { .frame { animation: none; } }
  .pill {
    position: fixed; left: 50%; bottom: 16px; transform: translateX(-50%);
    display: flex; align-items: center; gap: 10px; max-width: min(560px, calc(100vw - 32px));
    padding: 6px 6px 6px 14px; border-radius: 999px; pointer-events: auto;
    background: rgba(20, 20, 22, 0.92); color: #f5f5f5; box-shadow: 0 4px 18px rgba(0, 0, 0, 0.35);
    font: 500 13px/1.3 system-ui, -apple-system, 'Segoe UI', sans-serif;
  }
  .pill.quiet { padding-right: 14px; }
  .dot { width: 8px; height: 8px; border-radius: 50%; background: var(--glow); flex: none; }
  .text { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  button {
    all: unset; cursor: pointer; padding: 5px 12px; border-radius: 999px; flex: none;
    font: 600 12px/1.2 system-ui, -apple-system, 'Segoe UI', sans-serif;
    background: rgba(255, 255, 255, 0.12); color: #f5f5f5;
  }
  button:hover { background: rgba(255, 255, 255, 0.2); }
  button:focus-visible { outline: 2px solid var(--glow); outline-offset: 1px; }
  button.primary { background: var(--glow); color: #1a1a1a; }
`;

let host: HTMLElement | null = null;
let root: ShadowRoot | null = null;
let shown = '';

/** The frame and the Stop pill on a page a run works in, in a closed shadow root the page can neither style nor read. */
export function showGlow(command: Busy): void {
  const key = JSON.stringify([command.glow, command.waiting]);
  if (host?.isConnected && key === shown) return;
  shown = key;
  if (!host?.isConnected) mount();
  const color = command.waiting ? WAITING_COLOR : INDICATOR_COLOR;
  host!.style.setProperty('--glow', color);
  root!.replaceChildren(style(), ...(command.glow ? [element('div', 'frame')] : []), pill(command));
}

export function hideGlow(): void {
  host?.remove();
  host = null;
  root = null;
  shown = '';
}

/** Hidden, not removed, while a screenshot is taken. */
export function hushGlow(hushed: boolean): void {
  if (host) host.style.display = hushed ? 'none' : '';
}

function mount(): void {
  document.getElementById(HOST_ID)?.remove();
  host = document.createElement('div');
  host.id = HOST_ID;
  host.setAttribute(OVERLAY_ATTRIBUTE, '');
  host.style.cssText = 'all: initial; position: fixed; inset: 0; z-index: 2147483646; pointer-events: none;';
  root = host.attachShadow({ mode: 'closed' });
  (document.body ?? document.documentElement).append(host);
}

function pill({ waiting }: Busy): HTMLElement {
  const box = element('div', 'pill');
  box.setAttribute('role', 'status');
  const text = element('span', 'text');
  text.textContent = waiting ? waiting.text : 'Vexdom is working';
  box.append(element('span', 'dot'), text);
  if (waiting && !waiting.handOff) {
    box.classList.add('quiet');
    return box;
  }
  if (waiting?.handOff) box.append(button('Done — continue', 'primary', { op: 'handBack', allow: true }));
  box.append(button('Stop', '', { op: 'stopRun' }));
  return box;
}

function button(
  label: string,
  className: string,
  request: { op: 'stopRun' } | { op: 'handBack'; allow: boolean },
): HTMLButtonElement {
  const control = element('button', className);
  control.type = 'button';
  control.textContent = label;
  control.addEventListener('click', (event) => {
    // A page script can dispatch a click; only the user's own counts.
    if (!event.isTrusted) return;
    control.disabled = true;
    void browser.runtime.sendMessage({ channel: BRIDGE_CHANNEL, ...request }).catch(() => undefined);
  });
  return control;
}

function style(): HTMLStyleElement {
  const sheet = document.createElement('style');
  sheet.textContent = STYLE;
  return sheet;
}

function element<K extends keyof HTMLElementTagNameMap>(tag: K, className: string): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  if (className) node.className = className;
  return node;
}
