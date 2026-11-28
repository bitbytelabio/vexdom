import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { fakeBrowser } from 'wxt/testing';
import { dispatchClick, dispatchDrag, INPUT_ACK_MS } from './trusted-input';

const sent: { type?: string }[] = [];
const fakeDebugger = {
  sendCommand: vi.fn(async (_source: unknown, _method: string, params?: { type?: string }) => {
    sent.push(params ?? {});
    // A page whose main thread is stuck once the button is down: Chrome never answers the move.
    if (params?.type === 'mouseMoved' && sent.some((event) => event.type === 'mousePressed')) {
      return new Promise(() => {});
    }
    return {};
  }),
};

beforeAll(() => {
  Object.defineProperty(fakeBrowser, 'debugger', { value: fakeDebugger, configurable: true });
});

beforeEach(() => {
  sent.length = 0;
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
});

describe('a trusted drag', () => {
  it('fails within the input deadline when the page stops taking events, and lets the button go', async () => {
    const drag = dispatchDrag(
      { tabId: 1 },
      { approach: { x: 0, y: 0 }, grip: { x: 10, y: 10 }, drop: { x: 90, y: 90 }, steps: 4, holdMs: 0, settleMs: 0 },
    );
    const failed = expect(drag).rejects.toThrow(/did not take the mouseMoved event within 5 s/);
    await vi.advanceTimersByTimeAsync(INPUT_ACK_MS + 1_000);
    await failed;
    expect(sent.at(-1)?.type).toBe('mouseReleased');
  });
});

describe('a trusted click', () => {
  it('whose press the page never takes still lets the button go, once', async () => {
    fakeDebugger.sendCommand.mockImplementation(async (_source, _method, params) => {
      sent.push(params ?? {});
      return params?.type === 'mousePressed' ? new Promise(() => {}) : {};
    });
    const click = dispatchClick(
      { tabId: 1 },
      {
        point: { x: 5, y: 5 },
        from: { x: 0, y: 0 },
        button: 'left',
        clickCount: 1,
        modifiers: [],
        moveSteps: 1,
        hoverMs: 0,
        holdMs: 0,
      },
    );
    const failed = expect(click).rejects.toThrow(/did not take the mousePressed event/);
    await vi.advanceTimersByTimeAsync(INPUT_ACK_MS + 1_000);
    await failed;
    expect(sent.filter((event) => event.type === 'mouseReleased')).toHaveLength(1);
  });
});
