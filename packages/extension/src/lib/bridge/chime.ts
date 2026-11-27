import { browser } from 'wxt/browser';
import { CHIME_CHANNEL, playTone, type ChimeKind } from '@/lib/chime/tone';
import { readSetting, RUN_SOUND_KEY } from '@/lib/settings/run-surface';

const CHIME_PAGE_MS = 1_500;

async function offscreenPath(): Promise<string | null> {
  const [open] = await browser.runtime.getContexts({ contextTypes: [browser.runtime.ContextType.OFFSCREEN_DOCUMENT] });
  return open?.documentUrl ? new URL(open.documentUrl).pathname : open ? '' : null;
}

/**
 * Plays a short chime with a notice. Firefox's background is a page and plays it itself; Chromium's
 * service worker has no audio, so an open offscreen page plays it, or a chime page opened for it.
 * Best effort: a sound that cannot play leaves the notice as it was.
 */
export async function chime(kind: ChimeKind): Promise<void> {
  try {
    if (!(await readSetting(RUN_SOUND_KEY))) return;
    if (import.meta.env.FIREFOX) {
      await playTone(kind);
      return;
    }
    if ((await offscreenPath()) !== null) {
      await browser.runtime.sendMessage({ channel: CHIME_CHANNEL, kind });
      return;
    }
    await browser.offscreen.createDocument({
      url: `chime.html?kind=${kind}`,
      reasons: [browser.offscreen.Reason.AUDIO_PLAYBACK],
      justification: 'Plays a short sound when a Vexdom run finishes or needs you.',
    });
    setTimeout(() => {
      void offscreenPath()
        .then((path) => (path?.endsWith('/chime.html') ? browser.offscreen.closeDocument() : undefined))
        .catch(() => undefined);
    }, CHIME_PAGE_MS);
  } catch {
    // Sound is a nicety; the notice already went out.
  }
}
