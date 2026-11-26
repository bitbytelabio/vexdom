import { afterEach, describe, expect, it, vi } from 'vitest';
import { OVERLAY_ATTRIBUTE } from '@/lib/overlay';
import { pickWithLens, type LensOutcome } from './lens';

function stage() {
  const button = document.createElement('button');
  button.textContent = 'Sign in';
  document.body.append(button);
  const orb = document.createElement('div');
  orb.setAttribute(OVERLAY_ATTRIBUTE, '');
  document.documentElement.append(orb);
  return { button, orb };
}

function pointAt(target: Element) {
  vi.spyOn(document, 'elementFromPoint').mockReturnValue(target);
  window.dispatchEvent(new PointerEvent('pointermove', { clientX: 10, clientY: 10 }));
}

function clickAt(target: Element) {
  vi.spyOn(document, 'elementFromPoint').mockReturnValue(target);
  window.dispatchEvent(new MouseEvent('click', { clientX: 10, clientY: 10, bubbles: true, cancelable: true }));
}

function settled(pick: Promise<LensOutcome>) {
  let outcome: LensOutcome | null = null;
  void pick.then((value) => (outcome = value));
  return () => outcome;
}

describe('pickWithLens', () => {
  afterEach(() => {
    vi.restoreAllMocks();
    document.body.innerHTML = '';
    for (const host of document.querySelectorAll(`[${OVERLAY_ATTRIBUTE}]`)) host.remove();
  });

  it('picks the page element under the click', async () => {
    const { button } = stage();
    const pick = pickWithLens({ timeoutMs: 5_000 });
    pointAt(button);
    clickAt(button);
    await expect(pick).resolves.toEqual({ picked: button });
  });

  it('never picks an overlay the extension drew, even right after hovering the page', async () => {
    const { button, orb } = stage();
    const pick = pickWithLens({ timeoutMs: 5_000 });
    const outcome = settled(pick);

    pointAt(button);
    pointAt(orb);
    clickAt(orb);
    await Promise.resolve();
    expect(outcome()).toBeNull();

    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }));
    await expect(pick).resolves.toEqual({ cancelled: true });
  });

  it('keeps a finger’s touches from the page, whose handlers could otherwise cancel the tap', async () => {
    const { button } = stage();
    const heard = vi.fn();
    document.addEventListener('touchstart', heard, { capture: true });
    document.addEventListener('touchend', heard, { capture: true });
    const pick = pickWithLens({ timeoutMs: 5_000 });
    button.dispatchEvent(new Event('touchstart', { bubbles: true }));
    button.dispatchEvent(new Event('touchend', { bubbles: true }));
    expect(heard).not.toHaveBeenCalled();
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }));
    await expect(pick).resolves.toEqual({ cancelled: true });
    document.removeEventListener('touchstart', heard, { capture: true });
    document.removeEventListener('touchend', heard, { capture: true });
  });
});
