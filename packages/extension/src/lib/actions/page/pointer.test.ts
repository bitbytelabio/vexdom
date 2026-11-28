import { afterEach, describe, expect, it } from 'vitest';
import { OVERLAY_ATTRIBUTE } from '@/lib/overlay';
import { assertUncovered, elementAt } from './pointer';

function stage() {
  const footer = document.createElement('button');
  footer.textContent = 'Accept cookies';
  document.body.append(footer);
  const orb = document.createElement('div');
  orb.setAttribute(OVERLAY_ATTRIBUTE, '');
  document.documentElement.append(orb);
  return { footer, orb };
}

/* happy-dom lays nothing out, so what stands at a point is whatever the test stacks there. */
function stacked(...elements: Element[]) {
  document.elementsFromPoint = () => elements;
}

describe('elementAt', () => {
  afterEach(() => {
    document.body.innerHTML = '';
    for (const host of document.querySelectorAll(`[${OVERLAY_ATTRIBUTE}]`)) host.remove();
  });

  it('looks through the hands-free mic to the page beneath it', () => {
    const { footer, orb } = stage();
    stacked(orb, footer, document.body);
    expect(elementAt({ x: 400, y: 700 })).toBe(footer);
    expect(() => assertUncovered(footer, { x: 400, y: 700 })).not.toThrow();
  });

  it('still reports what the page itself put over an element', () => {
    const { footer } = stage();
    const banner = document.createElement('div');
    banner.id = 'banner';
    document.body.append(banner);
    stacked(banner, footer, document.body);
    expect(() => assertUncovered(footer, { x: 400, y: 700 })).toThrow(/covers \(400, 700\)/);
  });

  it('lets a click through a button that takes no pointer events to the chip around it', () => {
    const chip = document.createElement('div');
    const button = document.createElement('button');
    button.textContent = 'Comedy';
    chip.append(button);
    document.body.append(chip);
    stacked(chip, document.body);
    expect(() => assertUncovered(button, { x: 10, y: 10 })).toThrow(/covers \(10, 10\)/);
    button.style.pointerEvents = 'none';
    expect(() => assertUncovered(button, { x: 10, y: 10 })).not.toThrow();
    // A blocked form: the button inherits `none` and waits until the form lets go.
    chip.style.pointerEvents = 'none';
    expect(() => assertUncovered(button, { x: 10, y: 10 })).toThrow(/covers \(10, 10\)/);
  });

  it('looks into an open shadow root, where the document reports only the host', () => {
    const host = document.createElement('x-pay');
    document.body.append(host);
    const root = host.attachShadow({ mode: 'open' });
    const button = document.createElement('button');
    root.append(button);
    stacked(host, document.body);
    root.elementsFromPoint = () => [button, host];
    expect(elementAt({ x: 10, y: 10 })).toBe(button);
    expect(() => assertUncovered(button, { x: 10, y: 10 })).not.toThrow();
  });
});
