import { browser } from 'wxt/browser';
import { z } from 'zod';
import { invokeInFrame, invokeInTab } from '@/lib/actions/client';
import { ActionError } from '@/lib/actions/core';
import { attachFile } from '@/lib/actions/page/attach-file';
import { awaitMonitor } from '@/lib/actions/page/await-monitor';
import { callSiteTool } from '@/lib/actions/page/call-site-tool';
import { captureDownload } from '@/lib/actions/page/capture-download';
import { closeTab } from '@/lib/actions/page/close-tab';
import { cookies } from '@/lib/actions/page/cookies';
import { dragElement } from '@/lib/actions/page/drag-element';
import { evaluate } from '@/lib/actions/page/evaluate';
import { findCaptcha } from '@/lib/actions/page/find-captcha';
import { getPageInfo } from '@/lib/actions/page/get-page-info';
import { injectCode } from '@/lib/actions/page/inject-code';
import { listFiles } from '@/lib/actions/page/list-files';
import { listRecordings } from '@/lib/actions/page/list-recordings';
import { listSiteTools } from '@/lib/actions/page/list-site-tools';
import { cancelSiteTool } from '@/lib/actions/page/cancel-site-tool';
import { siteToolResult } from '@/lib/actions/page/site-tool-result';
import { monitorStatus } from '@/lib/actions/page/monitor-status';
import { navigate, resolveNavigation, type NavigateInput } from '@/lib/actions/page/navigate';
import { openTab } from '@/lib/actions/page/open-tab';
import { openWindow } from '@/lib/actions/page/open-window';
import { resizeWindow } from '@/lib/actions/page/resize-window';
import { pickElement } from '@/lib/actions/page/pick-element';
import { pushState } from '@/lib/actions/page/push-state';
import { query } from '@/lib/actions/page/query';
import { readConsole } from '@/lib/actions/page/read-console';
import { readNetwork } from '@/lib/actions/page/read-network';
import { readRecording } from '@/lib/actions/page/read-recording';
import { runCode } from '@/lib/actions/page/run-code';
import { screenshot } from '@/lib/actions/page/screenshot';
import { snapshot } from '@/lib/actions/page/snapshot';
import { searchSite } from '@/lib/actions/page/search-site';
import { solveCaptcha } from '@/lib/actions/page/solve-captcha';
import { startDiagnostics } from '@/lib/actions/page/start-diagnostics';
import { startMonitor } from '@/lib/actions/page/start-monitor';
import { startTimer } from '@/lib/actions/page/start-timer';
import { stopDiagnostics } from '@/lib/actions/page/stop-diagnostics';
import { stopMonitor } from '@/lib/actions/page/stop-monitor';
import { stopTimer } from '@/lib/actions/page/stop-timer';
import { switchFrame } from '@/lib/actions/page/switch-frame';
import { switchTab } from '@/lib/actions/page/switch-tab';
import { timerStatus } from '@/lib/actions/page/timer-status';
import { trustedClick } from '@/lib/actions/page/trusted-click';
import { waitFor } from '@/lib/actions/page/wait-for';
import { checkSites } from '@/lib/actions/page/check-sites';
import { dialog } from '@/lib/actions/page/dialog';
import { emulate } from '@/lib/actions/page/emulate';
import { harStart } from '@/lib/actions/page/har-start';
import { harStop } from '@/lib/actions/page/har-stop';
import { initScript } from '@/lib/actions/page/init-script';
import { mouse } from '@/lib/actions/page/mouse';
import { routeNetwork } from '@/lib/actions/page/route-network';
import { tabKeep } from '@/lib/actions/page/tab-keep';
import { unrouteNetwork } from '@/lib/actions/page/unroute-network';
import { pressKey } from '@/lib/actions/page/press-key';
import { typeText } from '@/lib/actions/page/type-text';
import { pdf } from '@/lib/actions/page/pdf';
import { recordStart } from '@/lib/actions/page/record-start';
import { recordStop } from '@/lib/actions/page/record-stop';
import { traceStart } from '@/lib/actions/page/trace-start';
import { traceStop } from '@/lib/actions/page/trace-stop';
import { vitals } from '@/lib/actions/page/vitals';
import { react } from '@/lib/actions/page/react';
import { a11yAudit } from '@/lib/actions/page/a11y-audit';
import { failure, success, type ActionResult } from '@/lib/actions/protocol';
import { EXPIRED_MESSAGE, REFUSED_MESSAGE, releaseForAction, sealForPage } from '@/lib/bridge/secret-vault';
import { findCaptchaInTab, solveCaptchaInTab } from '@/lib/bridge/captcha';
import { installToolkit, runToolkit, tabPlace } from '@/lib/bridge/code-toolkit';
import {
  callSiteToolInTab,
  cancelSiteToolCall,
  listSiteToolsInTab,
  pageInfoWithSiteTools,
  siteToolOutcome,
} from '@/lib/bridge/site-tools';
import { captureFromPage } from '@/lib/bridge/downloads';
import { cookiesFor } from '@/lib/bridge/cookies';
import { evaluateInTab } from '@/lib/bridge/evaluate';
import { pushStateInTab } from '@/lib/bridge/main-world';
import { waitForInTab } from '@/lib/bridge/wait-for';
import { focusedUrl, forgetFrameFocus, framePath, TOP_FRAME } from '@/lib/bridge/frame-focus';
import { blockedRefusal, refusalFor, refusalForTab, siteCheck, tabUrls } from '@/lib/bridge/site-guard';
import { switchFrameInTab } from '@/lib/bridge/frames';
import { refFrameOf, screenshotWithLabels, snapshotTab } from '@/lib/bridge/snapshot-frames';
import { listMeta, readBytes } from '@/lib/bridge/file-store';
import { readConsoleFor, readNetworkFor, startTabDiagnostics, stopTabDiagnostics } from '@/lib/bridge/diagnostics';
import { awaitMonitorDone, monitorStatusFor, startTabMonitor, stopTabMonitor } from '@/lib/bridge/monitor';
import { startJobTimer, stopJobTimer, timerStatusFor } from '@/lib/bridge/timer';
import {
  listRecordings as listStoredMeta,
  readRecordingBody,
  recordedOnBlockedSite,
} from '@/lib/bridge/recording-store';
import { pickInTab } from '@/lib/bridge/pick';
import { cued } from '@/lib/bridge/action-cues';
import { cueFor } from '@/lib/cues/plan';
import type { CuePlan } from '@/lib/cues/events';
import { dragInTab, mouseInTab, trustedClickInTab, trustedKeysInTab } from '@/lib/bridge/trusted-input';
import { routeNetworkInTab, unrouteNetworkInTab } from '@/lib/bridge/network-route';
import { startHar, stopHar } from '@/lib/bridge/har';
import { dialogInTab } from '@/lib/bridge/dialogs';
import { emulateInTab } from '@/lib/bridge/emulation';
import { initScriptInTab } from '@/lib/bridge/init-scripts';
import { pdfInTab, startRecording, startTrace, stopRecording, stopTrace } from '@/lib/bridge/capture';
import { a11yInTab, reactInTab, vitalsInTab } from '@/lib/bridge/devtools';
import { tabKeepInTab, thawIfFrozen } from '@/lib/bridge/tab-keep';
import { keepWatching, watchRequests } from '@/lib/bridge/request-watch';
import { closeOpenTab, openNewTab, switchToTab, watchForLoad, type TabRef } from '@/lib/bridge/tabs';
import { adoptSubtab, sessionForRun, sessionForTab, setCurrentTab, type TabSession } from '@/lib/bridge/tab-sessions';
import { adoptMcpTab } from '@/lib/bridge/mcp-tabs';
import { LOOKS, OUTSIDE_CALLER, PAGE_CHANGERS, rememberSeen, seenHost } from '@/lib/bridge/seen-sites';
import { rosterUpdate } from '@/lib/bridge/roster';

/**
 * The sanitizer's client-side half, wrapped around every action the harness runs.
 *
 * In: a sealed handle becomes plaintext only if it sits in a field that types into the
 * page, and only one hop before the content script gets it. Out: whatever the page gave
 * back is scanned and sealed before it can reach the daemon, so a credential read from a
 * page never crosses the socket at all.
 */
export async function invokeForHarness(
  action: string,
  input?: unknown,
  tabId?: number,
  runId?: string,
): Promise<ActionResult> {
  const refused = await guardTarget(action, input, tabId, runId);
  if (refused) return refused;

  const release = await releaseForAction(action, input);
  if (release.refused) return failure('SECRET_NOT_RELEASABLE', REFUSED_MESSAGE);
  if (release.unresolved) return failure('SECRET_EXPIRED', EXPIRED_MESSAGE);

  const result = await dispatch(action, release.input, tabId, runId, cueFor(action, input));
  const landed = await guardLanding(action, result, tabId, runId);
  if (landed) return landed;
  if (result.ok) await noteSeen(action, result.data, tabId, runId);
  // Cookies read for their own url are that site's, not the tab's.
  const ownUrl = cookiesUrl(action, input);
  const { value } = await sealForPage(result, ownUrl ? hostOf(ownUrl) : await targetOrigin(tabId, runId));
  // The Bridge's own address check asks page.query and reads only its value, so the roster waits for the next result.
  return action === query.name ? value : withRoster(value, runId);
}

/** What the caller has now seen: a page it looked at, or the one it moved to itself (a frame included). */
async function noteSeen(action: string, data: unknown, tabId?: number, runId?: string): Promise<void> {
  const moved = [navigate.name, openTab.name, switchTab.name, switchFrame.name].includes(action);
  if (!LOOKS.has(action) && !moved) return;
  const reported = data as { tabId?: unknown; activeTabId?: unknown } | undefined;
  const landedOn =
    action === openTab.name ? reported?.tabId : action === switchTab.name ? reported?.activeTabId : undefined;
  const owner = runId ? await sessionForRun(runId) : null;
  const tab = typeof landedOn === 'number' ? await pinnedTab(landedOn) : await resolveTab(tabId, owner);
  if (tab?.id == null) return;
  const url = await focusedUrl(tab.id, tab.url);
  await rememberSeen(tab.id, url ? hostOf(url) : undefined, owner?.sessionId ?? OUTSIDE_CALLER);
}

/** A conversation whose tabs changed mid-turn hears of it with the next result it gets. */
async function withRoster(value: ActionResult, runId?: string): Promise<ActionResult> {
  const owner = runId ? await sessionForRun(runId) : null;
  if (!owner || !value.ok || !value.data || typeof value.data !== 'object' || Array.isArray(value.data)) return value;
  const tabsNow = await rosterUpdate(owner);
  return tabsNow ? { ...value, data: { ...value.data, tabsNow } } : value;
}

/** Answered from extension storage alone — they never touch a tab, and what they hand back is filtered where it is kept. */
const TAB_FREE = new Set(
  [
    listFiles,
    listRecordings,
    readRecording,
    readConsole,
    readNetwork,
    stopDiagnostics,
    monitorStatus,
    stopMonitor,
    awaitMonitor,
    startTimer,
    timerStatus,
    stopTimer,
    siteToolResult,
    cancelSiteTool,
    checkSites,
    // Moves the window, never reads or acts on its page.
    resizeWindow,
  ].map((action) => action.name),
);

/** These pick another tab to land on or close, and check that tab where they pick it — in tabs.ts. */
const PICKS_ITS_TAB = new Set([openTab, openWindow, switchTab, closeTab].map((action) => action.name));

/** `page.cookies` with its own `url` needs no tab: that address is the one checked. */
const cookiesUrl = (action: string, input: unknown): string | undefined => {
  const url = action === cookies.name ? (input as { url?: unknown } | undefined)?.url : undefined;
  return typeof url === 'string' && url ? url : undefined;
};

const WATCHES_A_TAB = new Set([startMonitor, startDiagnostics].map((action) => action.name));

/**
 * The blocked-sites gate. Every action is checked unless it is named above, so an action
 * added later is covered without anyone remembering to. The checks are the tab's address,
 * where it is about to go, every frame on the focused path, and wherever the action itself
 * would send the browser.
 */
async function guardTarget(
  action: string,
  input: unknown,
  tabId?: number,
  runId?: string,
): Promise<ActionResult | null> {
  if (TAB_FREE.has(action) || PICKS_ITS_TAB.has(action)) return null;
  const ownUrl = cookiesUrl(action, input);
  if (ownUrl) return refusalFor(ownUrl);
  const owner = runId ? await sessionForRun(runId) : null;

  if (WATCHES_A_TAB.has(action)) {
    const watched = (input as { tabId?: unknown } | undefined)?.tabId;
    const tab = typeof watched === 'number' ? await pinnedTab(watched) : await resolveTab(tabId, owner);
    return refusalFor(...tabUrls(tab));
  }

  const tab = await resolveTab(tabId, owner);
  if (tab?.id == null) return null;
  if (action !== navigate.name && !isWebPage(tab.url || tab.pendingUrl)) {
    return failure(
      'TAB_UNREACHABLE',
      `Tab ${tab.id} is a browser page, not a web page — page actions cannot read or act on it. page.navigate to an absolute http(s) URL first.`,
    );
  }
  if (PAGE_CHANGERS.has(action)) {
    const liveUrl = await focusedUrl(tab.id, tab.url);
    const live = liveUrl ? hostOf(liveUrl) : undefined;
    const seen = await seenHost(tab.id, owner?.sessionId ?? OUTSIDE_CALLER);
    if (seen && live && seen !== live) {
      return failure(
        'PAGE_CHANGED',
        `Tab ${tab.id} is on ${live} now, not ${seen} where you last looked. Look at it again (snapshot) before acting.`,
      );
    }
  }
  const frames = (await framePath(tab.id)).map((step) => step.url);
  return refusalFor(...tabUrls(tab), ...frames, ...destinationsOf(action, input, tab.url));
}

/** A redirect, a form or a slow screenshot can leave the tab somewhere blocked; nothing read there goes back. */
async function guardLanding(
  action: string,
  result: ActionResult,
  tabId?: number,
  runId?: string,
): Promise<ActionResult | null> {
  if (!result.ok || TAB_FREE.has(action) || WATCHES_A_TAB.has(action)) return null;
  if (action === switchTab.name || action === closeTab.name) return null;
  const reported = result.data as { tabId?: unknown; finalUrl?: unknown; landedOn?: unknown } | undefined;

  if (action === openTab.name || action === openWindow.name) {
    return typeof reported?.tabId === 'number' ? refusalFor(...tabUrls(await pinnedTab(reported.tabId))) : null;
  }
  const owner = runId ? await sessionForRun(runId) : null;
  const tab = await resolveTab(tabId, owner);
  const reportedUrls = [textOf(reported?.finalUrl), textOf(reported?.landedOn)];
  return tab?.id != null ? refusalForTab(tab.id, ...reportedUrls) : refusalFor(...reportedUrls);
}

function destinationsOf(action: string, input: unknown, base?: string): (string | undefined)[] {
  if (action === navigate.name) {
    const parsed = navigate.input.safeParse(input ?? {});
    if (!parsed.success) return [];
    try {
      const plan = resolveNavigation(parsed.data as NavigateInput, base);
      return plan.kind === 'url' ? [plan.href] : [];
    } catch {
      return [];
    }
  }
  if (action === captureDownload.name || action === pushState.name) {
    const url = (input as { url?: unknown } | undefined)?.url;
    return typeof url === 'string' ? [absoluteUrl(url, base)] : [];
  }
  return [];
}

function absoluteUrl(url: string, base?: string): string | undefined {
  try {
    return new URL(url, base).href;
  } catch {
    return undefined;
  }
}

const textOf = (value: unknown): string | undefined => (typeof value === 'string' ? value : undefined);

const isWebPage = (url: string | undefined): boolean => !!url && /^https?:/i.test(url);

async function targetOrigin(tabId?: number, runId?: string): Promise<string | undefined> {
  const owner = runId ? await sessionForRun(runId) : null;
  const tab = await resolveTab(tabId, owner);
  const url = tab?.id != null ? await focusedUrl(tab.id, tab.url) : tab?.url;
  return url ? hostOf(url) : undefined;
}

function hostOf(url: string): string | undefined {
  try {
    return new URL(url).hostname.toLowerCase() || undefined;
  } catch {
    return undefined;
  }
}

async function dispatch(
  action: string,
  input: unknown,
  tabId: number | undefined,
  runId: string | undefined,
  cue: CuePlan,
): Promise<ActionResult> {
  if (action === listFiles.name) return listStoredFiles(input);
  if (action === listRecordings.name) return listStoredRecordings(input);
  if (action === readRecording.name) return readStoredRecording(input);
  if (action === siteToolResult.name) return siteToolOutcome(input);
  if (action === cancelSiteTool.name) return cancelSiteToolCall(input);
  if (action === checkSites.name) {
    const parsed = checkSites.input.safeParse(input ?? {});
    if (!parsed.success) return failure('INVALID_INPUT', z.prettifyError(parsed.error));
    return (await refusalFor(...parsed.data.urls)) ?? success({ allowed: true });
  }

  const owner = runId ? await sessionForRun(runId) : null;
  if (action === openTab.name) return openSessionTab(input, owner);
  if (action === openWindow.name) return openSessionWindow(input, owner);
  if (action === resizeWindow.name) return resizeSessionWindow(input, tabId, owner);
  const ownUrl = cookiesUrl(action, input);
  if (ownUrl) return cookiesFor(input);
  if (action === startDiagnostics.name) return beginDiagnostics(input, tabId ?? owner?.currentTabId, owner?.sessionId);
  if (action === readConsole.name) return readPageConsole(input);
  if (action === readNetwork.name) return readPageNetwork(input);
  if (action === stopDiagnostics.name) return stopRequestedDiagnostics(input);
  if (action === startMonitor.name) return beginMonitor(input, tabId ?? owner?.currentTabId);
  if (action === monitorStatus.name) return statusOfMonitors(input);
  if (action === stopMonitor.name) return stopRequestedMonitor(input);
  if (action === awaitMonitor.name) return awaitRequestedMonitor(input);
  if (action === startTimer.name) return beginTimer(input, owner?.sessionId);
  if (action === timerStatus.name) return statusOfTimers(input);
  if (action === stopTimer.name) return stopRequestedTimer(input);

  const tab = await resolveTab(tabId, owner);
  if (tab?.id == null) {
    if (owner) {
      return failure('SESSION_TAB_CLOSED', 'The tab this conversation was working in has been closed.');
    }
    return tabId == null
      ? failure('NO_ACTIVE_TAB', 'No active tab to control')
      : failure('MAPPING_TAB_CHANGED', 'The tab this run started in has been closed.');
  }
  const { id, windowId, url } = tab;
  return cued(id, cue, () => routeInTab({ id, windowId, url }, action, input, runId, owner)).finally(() =>
    keepWatching(id),
  );
}

async function routeInTab(
  tab: { id: number; windowId?: number; url?: string },
  action: string,
  input: unknown,
  runId: string | undefined,
  owner: TabSession | null,
): Promise<ActionResult> {
  if (action === tabKeep.name) return tabKeepInTab(tab.id, input);
  await thawIfFrozen(tab.id);
  // What the action fetches, its after-action snapshot waits for.
  if (action !== snapshot.name && action !== screenshot.name) watchRequests(tab.id);
  if (action === navigate.name)
    return isWebPage(tab.url) ? navigateTab(tab.id, input) : navigateViaTabsApi(tab.id, input);
  if (action === searchSite.name) return searchTab(tab.id, input);
  if (action === switchTab.name) return switchSessionTab({ id: tab.id, windowId: tab.windowId }, input, owner);
  if (action === switchFrame.name) return switchFrameInTab(tab.id, tab.url, input);
  if (action === closeTab.name) return closeOpenTab({ id: tab.id, windowId: tab.windowId }, input);
  if (action === screenshot.name) return screenshotWithLabels({ id: tab.id, windowId: tab.windowId }, input, runId);
  if (action === snapshot.name) return snapshotTab(tab.id, input);
  if (action === pickElement.name) return pickInTab({ id: tab.id, windowId: tab.windowId }, input);
  if (action === attachFile.name) return attachStoredFile(tab.id, input);
  if (action === captureDownload.name) return captureFromPage(tab.id, input);
  if (action === injectCode.name) return installToolkit(tabPlace(tab.id), await focusedUrl(tab.id, tab.url), input);
  if (action === runCode.name) return runToolkit(tabPlace(tab.id), await focusedUrl(tab.id, tab.url), input);
  if (action === getPageInfo.name) return pageInfoWithSiteTools(tab.id, tab.url, input);
  if (action === listSiteTools.name) return listSiteToolsInTab(tab.id, await focusedUrl(tab.id, tab.url), input);
  if (action === callSiteTool.name) return callSiteToolInTab(tab.id, await focusedUrl(tab.id, tab.url), input);
  if (action === trustedClick.name) return trustedClickInTab(tab.id, input);
  if (action === dragElement.name && isTrustedDrag(input)) return dragInTab(tab.id, input);
  if ((action === pressKey.name || action === typeText.name) && isTrustedDrag(input)) {
    return trustedKeysInTab(tab.id, action, input as Record<string, unknown>);
  }
  if (action === mouse.name) {
    const parsed = mouse.input.safeParse(input ?? {});
    return parsed.success ? mouseInTab(tab.id, parsed.data) : failure('INVALID_INPUT', z.prettifyError(parsed.error));
  }
  if (action === routeNetwork.name) return routeNetworkInTab(tab.id, input);
  if (action === unrouteNetwork.name) return unrouteNetworkInTab(tab.id, input);
  if (action === harStart.name) return startHar(tab.id, input);
  if (action === harStop.name) return stopHar(tab.id, input);
  if (action === dialog.name) return dialogInTab(tab.id, input);
  if (action === emulate.name) return emulateInTab(tab.id, input);
  if (action === initScript.name) return initScriptInTab(tab.id, input);
  if (action === pdf.name) return pdfInTab(tab.id, input);
  if (action === traceStart.name) return startTrace(tab.id, input);
  if (action === traceStop.name) return stopTrace(tab.id, input);
  if (action === recordStart.name) return startRecording(tab.id, input);
  if (action === recordStop.name) return stopRecording(tab.id);
  if (action === vitals.name) return vitalsInTab(tab.id, input);
  if (action === react.name) return reactInTab(tab.id, input);
  if (action === a11yAudit.name) return a11yInTab(tab.id, input);
  if (action === findCaptcha.name) return findCaptchaInTab(tab.id);
  if (action === solveCaptcha.name) return solveCaptchaInTab(tab.id, input);
  if (action === pushState.name) return pushStateInTab(tab.id, tab.url, input);
  if (action === waitFor.name) return waitForInTab(tab.id, input);
  if (action === cookies.name) return cookiesFor(input, tab.url);
  if (action === evaluate.name) return evaluateCode(tab.id, input);
  // A ref names its frame: the action goes there, whichever frame has the focus.
  const refFrame = refFrameOf(input);
  if (refFrame !== undefined) return invokeInFrame(tab.id, refFrame, action, input);
  return invokeInTab(tab.id, action, input);
}

/** `trusted: true`: a drag or key action dispatched through the debugger. */
function isTrustedDrag(input: unknown): boolean {
  return (input as { trusted?: unknown } | undefined)?.trusted === true;
}

async function pinnedTab(tabId: number) {
  try {
    return await browser.tabs.get(tabId);
  } catch {
    return undefined;
  }
}

async function resolveTab(tabId: number | undefined, owner: TabSession | null) {
  if (owner) return (await pinnedTab(owner.currentTabId)) ?? (await pinnedTab(owner.mainTabId));
  if (tabId != null) return pinnedTab(tabId);
  return (await browser.tabs.query({ active: true, currentWindow: true }))[0];
}

async function openSessionTab(input: unknown, owner: TabSession | null): Promise<ActionResult> {
  if (!owner) {
    const result = await openNewTab(input);
    const opened = result.ok ? (result.data as { tabId?: unknown }).tabId : undefined;
    if (typeof opened === 'number') await adoptMcpTab(opened);
    return result;
  }
  const anchor = (await pinnedTab(owner.currentTabId)) ?? (await pinnedTab(owner.mainTabId));
  const result = await openNewTab(input, anchor?.id != null ? { id: anchor.id, windowId: anchor.windowId } : undefined);
  if (!result.ok) return result;
  const opened = (result.data as { tabId?: unknown; activeTabId?: unknown }).tabId;
  if (typeof opened === 'number') {
    await adoptSubtab(owner.sessionId, opened, {
      activate: (result.data as { activeTabId?: unknown }).activeTabId === opened,
      opened: true,
    });
  }
  return result;
}

async function openSessionWindow(input: unknown, owner: TabSession | null): Promise<ActionResult> {
  const parsed = openWindow.input.safeParse(input ?? {});
  if (!parsed.success) return failure('INVALID_INPUT', z.prettifyError(parsed.error));
  let href: string | undefined;
  if (parsed.data.url) {
    try {
      const plan = resolveNavigation({ url: parsed.data.url } as NavigateInput);
      if (plan.kind !== 'url') return failure('INVALID_INPUT', 'page.openWindow opens a URL.');
      href = plan.href;
    } catch (error) {
      return error instanceof ActionError
        ? failure(error.code, error.message)
        : failure('ACTION_FAILED', String(error));
    }
    const refused = await refusalFor(href);
    if (refused) return refused;
  }
  let created;
  try {
    created = await browser.windows.create({ ...(href ? { url: href } : {}), focused: true });
  } catch (error) {
    return failure('ACTION_FAILED', error instanceof Error ? error.message : String(error));
  }
  const tabId = created?.tabs?.[0]?.id;
  if (created?.id == null || tabId == null)
    return failure('ACTION_FAILED', 'The browser opened a window but reported no tab in it.');
  if (owner) await adoptSubtab(owner.sessionId, tabId, { activate: true, opened: true });
  else await adoptMcpTab(tabId);
  return success({ windowId: created.id, tabId });
}

async function resizeSessionWindow(
  input: unknown,
  tabId: number | undefined,
  owner: TabSession | null,
): Promise<ActionResult> {
  const parsed = resizeWindow.input.safeParse(input ?? {});
  if (!parsed.success) return failure('INVALID_INPUT', z.prettifyError(parsed.error));
  const tab = await resolveTab(tabId, owner);
  if (tab?.windowId == null) return failure('NO_ACTIVE_TAB', 'No tab to find the window of.');
  const { state, width, height, left, top } = parsed.data;
  try {
    const before = await browser.windows.get(tab.windowId);
    if (state) {
      await browser.windows.update(tab.windowId, { state });
    } else {
      if (before.state !== 'normal') await browser.windows.update(tab.windowId, { state: 'normal' });
      const geometry = Object.fromEntries(
        Object.entries({ width, height, left, top }).filter(([, value]) => value !== undefined),
      );
      await browser.windows.update(tab.windowId, geometry);
    }
    const now = await browser.windows.get(tab.windowId);
    return success({
      performed: 'resized the window',
      windowId: now.id,
      width: now.width,
      height: now.height,
      left: now.left,
      top: now.top,
      state: now.state,
    });
  } catch (error) {
    return failure('ACTION_FAILED', error instanceof Error ? error.message : String(error));
  }
}

async function evaluateCode(tabId: number, input: unknown): Promise<ActionResult> {
  const parsed = evaluate.input.safeParse(input ?? {});
  if (!parsed.success) return failure('INVALID_INPUT', z.prettifyError(parsed.error));
  return evaluateInTab(tabId, parsed.data.code, parsed.data.timeoutMs);
}

async function switchSessionTab(current: TabRef, input: unknown, owner: TabSession | null): Promise<ActionResult> {
  if (!owner) return switchToTab(current, input);

  const wanted = (input ?? {}) as { tabId?: unknown };
  if (typeof wanted.tabId === 'number' && !owner.tabIds.includes(wanted.tabId)) {
    const other = await sessionForTab(wanted.tabId);
    if (other) {
      return failure(
        'TAB_IN_USE',
        `Another Vexdom conversation is working in tab ${wanted.tabId}. Leave it alone, or ask the user to end that session first.`,
      );
    }
  }

  const result = await switchToTab(current, input);
  if (!result.ok) return result;
  const landed = (result.data as { activeTabId?: unknown }).activeTabId;
  if (typeof landed !== 'number' || landed === current.id) return result;

  if (owner.tabIds.includes(landed)) await setCurrentTab(owner.sessionId, landed);
  else if (!(await sessionForTab(landed)))
    await adoptSubtab(owner.sessionId, landed, { activate: true, opened: false });
  return result;
}

async function beginDiagnostics(input: unknown, tabId?: number, owner?: string): Promise<ActionResult> {
  const parsed = startDiagnostics.input.safeParse(input ?? {});
  if (!parsed.success) return failure('INVALID_INPUT', z.prettifyError(parsed.error));
  return startTabDiagnostics(parsed.data, tabId, owner);
}

async function readPageConsole(input: unknown): Promise<ActionResult> {
  const parsed = readConsole.input.safeParse(input ?? {});
  if (!parsed.success) return failure('INVALID_INPUT', z.prettifyError(parsed.error));
  return readConsoleFor(parsed.data);
}

async function readPageNetwork(input: unknown): Promise<ActionResult> {
  const parsed = readNetwork.input.safeParse(input ?? {});
  if (!parsed.success) return failure('INVALID_INPUT', z.prettifyError(parsed.error));
  return readNetworkFor(parsed.data);
}

async function stopRequestedDiagnostics(input: unknown): Promise<ActionResult> {
  const parsed = stopDiagnostics.input.safeParse(input ?? {});
  if (!parsed.success) return failure('INVALID_INPUT', z.prettifyError(parsed.error));
  return stopTabDiagnostics(parsed.data.diagnosticsId);
}

async function beginMonitor(input: unknown, tabId?: number): Promise<ActionResult> {
  const parsed = startMonitor.input.safeParse(input ?? {});
  if (!parsed.success) return failure('INVALID_INPUT', z.prettifyError(parsed.error));
  return startTabMonitor(parsed.data, tabId);
}

async function statusOfMonitors(input: unknown): Promise<ActionResult> {
  const parsed = monitorStatus.input.safeParse(input ?? {});
  if (!parsed.success) return failure('INVALID_INPUT', z.prettifyError(parsed.error));
  return monitorStatusFor(parsed.data.monitorId);
}

async function stopRequestedMonitor(input: unknown): Promise<ActionResult> {
  const parsed = stopMonitor.input.safeParse(input ?? {});
  if (!parsed.success) return failure('INVALID_INPUT', z.prettifyError(parsed.error));
  return stopTabMonitor(parsed.data.monitorId);
}

async function awaitRequestedMonitor(input: unknown): Promise<ActionResult> {
  const parsed = awaitMonitor.input.safeParse(input ?? {});
  if (!parsed.success) return failure('INVALID_INPUT', z.prettifyError(parsed.error));
  return awaitMonitorDone(parsed.data.monitorId, parsed.data.timeoutMs);
}

async function beginTimer(input: unknown, sessionId?: string): Promise<ActionResult> {
  const parsed = startTimer.input.safeParse(input ?? {});
  if (!parsed.success) return failure('INVALID_INPUT', z.prettifyError(parsed.error));
  return startJobTimer(parsed.data, sessionId);
}

async function statusOfTimers(input: unknown): Promise<ActionResult> {
  const parsed = timerStatus.input.safeParse(input ?? {});
  if (!parsed.success) return failure('INVALID_INPUT', z.prettifyError(parsed.error));
  return timerStatusFor(parsed.data.timerId);
}

async function stopRequestedTimer(input: unknown): Promise<ActionResult> {
  const parsed = stopTimer.input.safeParse(input ?? {});
  if (!parsed.success) return failure('INVALID_INPUT', z.prettifyError(parsed.error));
  return stopJobTimer(parsed.data.timerId);
}

async function listStoredFiles(input: unknown): Promise<ActionResult> {
  const filter = (input as { nameContains?: unknown } | undefined)?.nameContains;
  const needle = typeof filter === 'string' ? filter.toLowerCase() : null;
  const files = (await listMeta())
    .filter((f) => !needle || f.name.toLowerCase().includes(needle))
    .map(({ id, name, mime, size, status, report, addedAt }) => ({
      id,
      name,
      mime,
      size,
      status,
      summary: report?.summary,
      addedAt,
    }));
  return success({ files });
}

async function listStoredRecordings(input: unknown): Promise<ActionResult> {
  const filters = input as { host?: unknown; nameContains?: unknown } | undefined;
  const host = typeof filters?.host === 'string' ? filters.host.toLowerCase() : null;
  const needle = typeof filters?.nameContains === 'string' ? filters.nameContains.toLowerCase() : null;
  const blocked = await siteCheck();
  const recordings = (await listStoredMeta())
    .filter((r) => !blocked(r.startUrl))
    .filter((r) => !host || r.host.toLowerCase() === host)
    .filter((r) => !needle || `${r.name} ${r.goal ?? ''}`.toLowerCase().includes(needle))
    .map(({ id, name, host: on, status, goal, summary, steps, capturedValues, durationMs, createdAt }) => ({
      id,
      name,
      host: on,
      status,
      goal,
      summary,
      steps,
      capturedValues,
      durationMs,
      createdAt,
    }));
  return success({ recordings });
}

async function readStoredRecording(input: unknown): Promise<ActionResult> {
  const id = (input as { recordingId?: unknown } | undefined)?.recordingId;
  if (typeof id !== 'string' || !id) {
    return failure('INVALID_INPUT', 'Provide the "recordingId" of a recording from page.listRecordings.');
  }
  const meta = (await listStoredMeta()).find((r) => r.id === id);
  if (!meta) return failure('RECORDING_NOT_FOUND', `No recording with id "${id}".`);
  const body = await readRecordingBody(id);
  if (await recordedOnBlockedSite(meta.startUrl, body?.events)) return blockedRefusal();
  if (!body?.workflow) {
    return failure(
      'RECORDING_NOT_READY',
      meta.status === 'error'
        ? `That recording could not be turned into steps: ${meta.error ?? 'unknown error'}`
        : 'That recording is still being turned into steps — try again shortly.',
    );
  }
  return success({
    id: meta.id,
    name: meta.name,
    host: meta.host,
    startUrl: meta.startUrl,
    capturedValues: meta.capturedValues,
    durationMs: meta.durationMs,
    goal: body.workflow.goal,
    summary: body.workflow.summary,
    variables: body.workflow.variables,
    caveats: body.workflow.caveats,
    steps: body.workflow.steps,
  });
}

/** A captured download or local files (`paths`) arrive with their bytes already filled in by the
 * daemon, which is the only side that has them; a stored file is resolved here, where the store lives. */
async function attachStoredFile(tabId: number, input: unknown): Promise<ActionResult> {
  const args = (input ?? {}) as { fileId?: unknown; target?: unknown; content?: unknown; files?: unknown };
  if ((typeof args.content === 'string' && args.content) || Array.isArray(args.files)) {
    return invokeInTab(tabId, attachFile.name, input);
  }

  const { fileId, target } = args;
  if (typeof fileId !== 'string' || !fileId) {
    return failure(
      'INVALID_INPUT',
      'attachFile needs a "fileId" from page.listFiles or a "downloadId" from page.captureDownload.',
    );
  }
  const bytes = await readBytes(fileId);
  if (!bytes) {
    return failure('FILE_NOT_FOUND', `No stored file with id "${fileId}". Call page.listFiles to see stored files.`);
  }
  return invokeInTab(tabId, attachFile.name, {
    fileId,
    target,
    name: bytes.name,
    mime: bytes.mime,
    content: bytes.content,
  });
}

const NO_CONTENT_SCRIPT = 'Receiving end does not exist';

async function navigateTab(tabId: number, input: unknown): Promise<ActionResult> {
  await forgetFrameFocus(tabId);
  const inPage = await invokeInFrame(tabId, TOP_FRAME, navigate.name, input);
  if (inPage.ok) {
    return success({ ...(inPage.data as object), loaded: await watchForLoad(tabId).settled });
  }
  if (inPage.error.code !== 'TAB_UNREACHABLE') return inPage;
  if (!inPage.error.message.includes(NO_CONTENT_SCRIPT)) {
    return success({ navigating: true, loaded: await watchForLoad(tabId).settled });
  }
  return navigateViaTabsApi(tabId, input);
}

async function navigateViaTabsApi(tabId: number, input: unknown): Promise<ActionResult> {
  const parsed = navigate.input.safeParse(input ?? {});
  if (!parsed.success) return failure('INVALID_INPUT', z.prettifyError(parsed.error));

  let plan;
  try {
    plan = resolveNavigation(parsed.data as NavigateInput);
  } catch (error) {
    return error instanceof ActionError ? failure(error.code, error.message) : failure('ACTION_FAILED', String(error));
  }

  const loaded = watchForLoad(tabId);
  try {
    if (plan.kind === 'url') await browser.tabs.update(tabId, { url: plan.href });
    else if (plan.action === 'reload') await browser.tabs.reload(tabId);
    else if (plan.action === 'back') await browser.tabs.goBack(tabId);
    else await browser.tabs.goForward(tabId);
  } catch (error) {
    loaded.cancel();
    const detail = error instanceof Error ? error.message : String(error);
    return failure(
      'ACTION_FAILED',
      plan.kind === 'history' && plan.action !== 'reload'
        ? `Cannot go ${plan.action}: this tab has no ${plan.action === 'back' ? 'previous' : 'next'} page in its history (${detail})`
        : detail,
    );
  }

  const completed = await loaded.settled;
  return success(
    plan.kind === 'url' ? { navigatedTo: plan.href, loaded: completed } : { performed: plan.action, loaded: completed },
  );
}

const FIELD_SETTLE_MS = 2_000;

async function searchTab(tabId: number, input: unknown): Promise<ActionResult> {
  const inPage = await invokeInTab(tabId, searchSite.name, input);
  if (!inPage.ok) return inPage;
  const searched = inPage.data as { via?: unknown; submitted?: unknown };
  const loaded = await settleAfterSearch(tabId, searched.via === 'url' || searched.submitted === true);
  return success({ ...(inPage.data as object), loaded, landedOn: (await pinnedTab(tabId))?.url });
}

async function settleAfterSearch(tabId: number, navigating: boolean): Promise<boolean> {
  const watcher = watchForLoad(tabId);
  if (navigating) return watcher.settled;
  const raced = await Promise.race([
    watcher.settled,
    new Promise<null>((resolve) => setTimeout(() => resolve(null), FIELD_SETTLE_MS)),
  ]);
  if (raced === null) watcher.cancel();
  return raced ?? false;
}
