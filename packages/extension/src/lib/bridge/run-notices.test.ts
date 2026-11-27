import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fakeBrowser } from 'wxt/testing';
import { RUN_NOTICES_KEY } from '@/lib/settings/run-surface';
import { announceRunEnd } from './run-notices';
import type { TabSession } from './tab-sessions';

vi.mock('./device-notices', () => ({
  browserInFront: vi.fn(async () => true),
  noticeOnDevice: vi.fn(async () => false),
}));
vi.mock('./toast', () => ({
  showToast: vi.fn(async () => false),
  showToastOn: vi.fn(async () => null),
  goToTab: vi.fn(),
  hideToast: vi.fn(),
}));
vi.mock('./chime', () => ({ chime: vi.fn(async () => undefined) }));

const session = (tabIds: number[]): TabSession => ({
  sessionId: 's1',
  mainTabId: tabIds[0],
  tabIds,
  userTabIds: tabIds,
  openedTabIds: [],
  currentTabId: tabIds[0],
  windowId: 1,
  title: 'GitHub',
  host: 'github.com',
  runId: null,
  turns: 1,
  createdAt: 0,
  lastActivityAt: 0,
});

let elsewhere: number;
let create: ReturnType<typeof vi.fn>;

beforeEach(async () => {
  fakeBrowser.reset();
  elsewhere = (await fakeBrowser.tabs.create({ url: 'https://example.com/', active: true })).id!;
  vi.spyOn(fakeBrowser.windows, 'getLastFocused').mockResolvedValue({ id: 1, focused: true, state: 'normal' } as never);
  vi.spyOn(fakeBrowser.tabs, 'query').mockImplementation(async () => [{ id: elsewhere } as never]);
  vi.spyOn(fakeBrowser.runtime, 'getManifest').mockReturnValue({ icons: { 128: 'icon/128.png' } } as never);
  create = vi.fn(async () => 'id');
  Object.assign(fakeBrowser, { notifications: { create, clear: vi.fn(), onClicked: { addListener: vi.fn() } } });
});

afterEach(() => vi.restoreAllMocks());

describe('announceRunEnd', () => {
  it('notifies with the headline while the user is on another tab', async () => {
    await announceRunEnd(session([100]), { outcome: 'ok', headline: '3 pull requests wait on you.' });
    expect(create).toHaveBeenCalledWith(
      'vexdom-run:s1',
      expect.objectContaining({ title: 'github.com', message: '3 pull requests wait on you.' }),
    );
  });

  it('stays quiet while the user is on one of the conversation’s tabs', async () => {
    await announceRunEnd(session([elsewhere]), { outcome: 'ok', headline: 'Done.' });
    expect(create).not.toHaveBeenCalled();
  });

  it('stays quiet for a stop the user asked for', async () => {
    await announceRunEnd(session([100]), { outcome: 'cancelled', reason: 'CANCELLED' });
    expect(create).not.toHaveBeenCalled();
  });

  it('stays quiet when notices are off', async () => {
    await fakeBrowser.storage.local.set({ [RUN_NOTICES_KEY]: false });
    await announceRunEnd(session([100]), { outcome: 'ok', headline: 'Done.' });
    expect(create).not.toHaveBeenCalled();
  });
});
