import { browser } from 'wxt/browser';
import { injectContentScript } from '@/lib/actions/client';
import { failure, success, type ActionResult } from '@/lib/actions/protocol';
import { REGION_CHANNEL, type RegionPick } from '@/lib/region/events';
import { cropViewport } from './pick';
import { refusalFor, tabUrls } from './site-guard';

const NO_CONTENT_SCRIPT = 'Receiving end does not exist';
const PAINT_SETTLE_MS = 120;
const MAX_SIDE = 2000;

export interface RegionCapture {
  name: string;
  mime: 'image/png';
  dataUrl: string;
  width: number;
  height: number;
}

async function askForBox(tabId: number): Promise<RegionPick | null> {
  const ask = () =>
    browser.tabs.sendMessage(
      tabId,
      { channel: REGION_CHANNEL, op: 'pick' },
      { frameId: 0 },
    ) as Promise<RegionPick | null>;
  try {
    return await ask();
  } catch (error) {
    if (!String(error).includes(NO_CONTENT_SCRIPT) || !(await injectContentScript(tabId))) throw error;
    return ask();
  }
}

/** The user draws a box on the tab's page; what is inside it comes back as a PNG to attach. No timeout: they are choosing. */
export async function captureRegion(tabId: number): Promise<ActionResult<RegionCapture>> {
  const tab = await browser.tabs.get(tabId).catch(() => null);
  if (!tab?.url || !/^https?:/i.test(tab.url)) return failure('TAB_UNREACHABLE', 'Vexdom cannot draw on this page.');
  const refused = await refusalFor(...tabUrls(tab));
  if (refused) return refused as ActionResult<RegionCapture>;

  let pick: RegionPick | null;
  try {
    pick = await askForBox(tabId);
  } catch (error) {
    return failure('TAB_UNREACHABLE', error instanceof Error ? error.message : String(error));
  }
  if (!pick) return failure('CANCELLED', 'No region was chosen.');

  await new Promise((resolve) => setTimeout(resolve, PAINT_SETTLE_MS));
  try {
    const shot = await cropViewport(
      tab.windowId,
      pick.rect,
      { dpr: pick.dpr, ...pick.viewport },
      { padding: 0, maxSide: MAX_SIDE, format: 'png' },
    );
    if (!shot) return failure('CANCELLED', 'No region was chosen.');
    const name = `region-${new Date().toISOString().replace(/[:.]/g, '-')}.png`;
    return success({ name, mime: 'image/png', ...shot });
  } catch (error) {
    return failure('CAPTURE_UNSUPPORTED', error instanceof Error ? error.message : String(error));
  }
}
