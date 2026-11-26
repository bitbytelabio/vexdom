import { browser } from 'wxt/browser';
import { LIST_UNREADABLE_MESSAGE, SITE_BLOCKED } from '@/lib/settings/blocked-sites';
import { dispatch } from './dispatch';
import { BRIDGE_CHANNEL, failure, isActionInvocation, type ActionInvocation, type ActionResult } from './protocol';

export function exposeActions() {
  browser.runtime.onMessage.addListener((message, _sender, sendResponse) => {
    if (!isActionInvocation(message)) return;
    void answer(message).then(sendResponse);
    return true;
  });
}

/**
 * The page's own word on where it is, checked again whatever the background already decided. The
 * background holds the matcher (ext-core's wasm, which a content script does not load), so it is
 * asked about this address; no answer refuses.
 */
async function pageRefusal(): Promise<ActionResult | null> {
  try {
    const refused = (await browser.runtime.sendMessage({ channel: BRIDGE_CHANNEL, op: 'siteRefusal', url: location.href })) as
      | ActionResult
      | null
      | undefined;
    return refused ?? null;
  } catch {
    return failure(SITE_BLOCKED, LIST_UNREADABLE_MESSAGE);
  }
}

async function answer({ action, input }: ActionInvocation): Promise<ActionResult> {
  return (await pageRefusal()) ?? dispatch(action, input);
}
