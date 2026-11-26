import { beforeEach, describe, expect, test, vi } from 'vitest';
import { fakeBrowser } from 'wxt/testing';

const calls = vi.hoisted(() => ({ invoked: [] as [number, unknown][], drawn: [] as [number, unknown][], cleared: [] as number[], shots: 0 }));

/** Frame 0 embeds frame 3 (slot at node 1), which embeds frame 7. */
const trees: Record<number, unknown> = {
  0: { v: 1, docKey: 'top', url: 'https://shop.test/', title: 'Shop', nodes: [], frames: [{ node: 1, frameId: 3, sameOrigin: false }] },
  3: { v: 1, docKey: 'pay', url: 'https://pay.test/', title: 'Pay', nodes: [], frames: [{ node: 2, frameId: 7, sameOrigin: false }] },
  7: { v: 1, docKey: 'deep', url: 'https://deep.test/', title: 'Deep', nodes: [], frames: [] },
};

/** Chrome: frames say which nonce their parent posted them. */
const claims = vi.hoisted(() => new Map<number, string | null>());

vi.mock('@/lib/actions/client', () => ({
  injectContentScript: vi.fn(async () => true),
  invokeInFrame: vi.fn(async (_tab: number, frameId: number, _action: string, input: unknown) => {
    calls.invoked.push([frameId, input]);
    return frameId in trees ? { ok: true, data: structuredClone(trees[frameId]) } : { ok: false, error: { code: 'TAB_UNREACHABLE', message: 'gone' } };
  }),
}));
vi.mock('./secret-vault', () => ({ sealForPage: async (value: unknown) => ({ value, found: [] }) }));
vi.mock('./screenshot', () => ({
  screenshotTab: vi.fn(async (_tab: unknown, input: unknown) => {
    calls.shots++;
    expect(calls.cleared).toEqual([]);
    return { ok: true, data: { input } };
  }),
}));
vi.mock('./frame-focus', () => ({
  TOP_FRAME: 0,
  CLAIM_GAP_MS: 0,
  describeFrame: async () => ({ url: '', title: '', top: false, viewport: { w: 0, h: 0 } }),
  claimOf: async (_tab: number, frameId: number) => claims.get(frameId) ?? null,
  drawLabels: vi.fn(async (_tab: number, frameId: number, labels: unknown) => calls.drawn.push([frameId, labels])),
  clearLabels: vi.fn(async (_tab: number, frameId: number) => calls.cleared.push(frameId)),
}));

const { claimFrame, refFrameOf, screenshotWithLabels, snapshotTab } = await import('./snapshot-frames');

beforeEach(() => {
  claims.clear();
  calls.invoked.length = calls.drawn.length = calls.cleared.length = 0;
  calls.shots = 0;
});

describe('a snapshot across frames', () => {
  test('hangs each frame’s own tree in its slot, two levels down, and says which tab it is', async () => {
    const result = await snapshotTab(42, { selector: '#main', interactive: true });
    expect(result.ok).toBe(true);
    const tree = (result as { data: { tabId: number; frames: { snapshot?: { docKey: string; frames: { snapshot?: { docKey: string } }[] } }[] } }).data;
    expect(tree.tabId).toBe(42);
    expect(tree.frames[0].snapshot?.docKey).toBe('pay');
    expect(tree.frames[0].snapshot?.frames[0].snapshot?.docKey).toBe('deep');
    // The selector scopes the document it was asked of; child frames are walked whole.
    expect(calls.invoked).toEqual([
      [0, { selector: '#main', interactive: true }],
      [3, { interactive: true, maxNodes: undefined, includeFrames: true }],
      [7, { interactive: true, maxNodes: undefined, includeFrames: true }],
    ]);
  });

  test('on Chrome, finds a frame by the nonce its parent posted, and never by one two frames claim', async () => {
    trees[0] = {
      ...(trees[0] as object),
      frames: [
        { node: 1, probe: 'nonce-pay', sameOrigin: false },
        { node: 2, probe: 'nonce-twice', sameOrigin: false },
      ],
    };
    claims.set(3, 'nonce-pay').set(8, 'nonce-twice').set(9, 'nonce-twice');
    const all = [0, 3, 7, 8, 9].map((frameId) => ({ frameId, documentId: '', result: true }));
    vi.spyOn(fakeBrowser.scripting, 'executeScript').mockResolvedValue(all as never);
    const result = await snapshotTab(42, { interactive: true });
    const tree = (result as { data: { frames: { frameId?: number; snapshot?: { docKey: string } }[] } }).data;
    expect(tree.frames[0]).toMatchObject({ frameId: 3, snapshot: { docKey: 'pay' } });
    expect(tree.frames[1].frameId).toBeUndefined();
    trees[0] = { ...(trees[0] as object), frames: [{ node: 1, frameId: 3, sameOrigin: false }] };
  });

  test('names the one frame that holds a nonce, and none when two hold it', async () => {
    claims.set(7, 'nonce-switch').set(8, 'nonce-twice').set(9, 'nonce-twice');
    const all = [0, 3, 7, 8, 9].map((frameId) => ({ frameId, documentId: '', result: true }));
    vi.spyOn(fakeBrowser.scripting, 'executeScript').mockResolvedValue(all as never);
    await expect(claimFrame(42, 'nonce-switch')).resolves.toBe(7);
    await expect(claimFrame(42, 'nonce-twice')).resolves.toBeUndefined();
  });

  test('leaves frames out when asked to', async () => {
    await snapshotTab(42, { includeFrames: false });
    expect(calls.invoked.map(([frameId]) => frameId)).toEqual([0]);
  });
});

describe('labelled screenshots', () => {
  test('each frame draws its own labels before the capture and clears them after', async () => {
    const labels = [
      { frameId: 0, doc: 'top', id: 1, label: 'e1' },
      { frameId: 3, doc: 'pay', id: 4, label: 'e2' },
    ];
    const result = await screenshotWithLabels({ id: 42 }, { format: 'png', labels }, undefined);
    expect(result).toEqual({ ok: true, data: { input: { format: 'png' } } });
    expect(calls.drawn).toEqual([
      [0, [labels[0]]],
      [3, [labels[1]]],
    ]);
    expect(calls.cleared.sort()).toEqual([0, 3]);
  });

  test('a screenshot without labels draws nothing', async () => {
    await screenshotWithLabels({ id: 42 }, { format: 'png' }, undefined);
    expect(calls.drawn).toEqual([]);
    expect(calls.shots).toBe(1);
  });
});

test('a ref target, source or destination names its frame', () => {
  expect(refFrameOf({ target: { ref: { doc: 'pay', id: 4, frameId: 3 } } })).toBe(3);
  expect(refFrameOf({ from: { ref: { doc: 'pay', id: 4, frameId: 5 } }, to: { selector: '#bin' } })).toBe(5);
  expect(refFrameOf({ target: { selector: '#go' } })).toBeUndefined();
});
