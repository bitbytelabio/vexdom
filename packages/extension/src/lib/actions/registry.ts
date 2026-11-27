import { z } from 'zod';
import type { AnyAction } from './core';
import type { ToolDescriptor } from './manifest';
import { pdf } from './page/pdf';
import { recordStart } from './page/record-start';
import { recordStop } from './page/record-stop';
import { traceStart } from './page/trace-start';
import { traceStop } from './page/trace-stop';
import { vitals } from './page/vitals';
import { react } from './page/react';
import { a11yAudit } from './page/a11y-audit';
import { checkSites } from './page/check-sites';
import { dialog } from './page/dialog';
import { emulate } from './page/emulate';
import { harStart } from './page/har-start';
import { harStop } from './page/har-stop';
import { initScript } from './page/init-script';
import { mouse } from './page/mouse';
import { routeNetwork } from './page/route-network';
import { tabKeep } from './page/tab-keep';
import { unrouteNetwork } from './page/unroute-network';
import { applyTheme } from './page/apply-theme';
import { attachFile } from './page/attach-file';
import { auditContrast } from './page/audit-contrast';
import { awaitMonitor } from './page/await-monitor';
import { callSiteTool } from './page/call-site-tool';
import { cancelSiteTool } from './page/cancel-site-tool';
import { captureDownload } from './page/capture-download';
import { clickElement } from './page/click-element';
import { clipboard } from './page/clipboard';
import { closeTab } from './page/close-tab';
import { cookies } from './page/cookies';
import { UNDESCRIBED } from './page/dom';
import { dragElement } from './page/drag-element';
import { evaluate } from './page/evaluate';
import { extractText } from './page/extract-text';
import { fillInput } from './page/fill-input';
import { findCaptcha } from './page/find-captcha';
import { findProgress } from './page/find-progress';
import { findSearch } from './page/find-search';
import { focusInput } from './page/focus-input';
import { getPageInfo } from './page/get-page-info';
import { highlightElement } from './page/highlight-element';
import { hoverElement } from './page/hover-element';
import { injectCode } from './page/inject-code';
import { insertText } from './page/insert-text';
import { listDownloads } from './page/list-downloads';
import { listFiles } from './page/list-files';
import { listRecordings } from './page/list-recordings';
import { listSiteTools } from './page/list-site-tools';
import { monitorStatus } from './page/monitor-status';
import { navigate } from './page/navigate';
import { openTab } from './page/open-tab';
import { openWindow } from './page/open-window';
import { resizeWindow } from './page/resize-window';
import { pickElement } from './page/pick-element';
import { pressKey } from './page/press-key';
import { pushState } from './page/push-state';
import { query } from './page/query';
import { readConsole } from './page/read-console';
import { readHtml } from './page/read-html';
import { readNetwork } from './page/read-network';
import { readRecording } from './page/read-recording';
import { readTheme } from './page/read-theme';
import { runCode } from './page/run-code';
import { screenshot } from './page/screenshot';
import { scrollTo } from './page/scroll-to';
import { searchSite } from './page/search-site';
import { selectOption } from './page/select-option';
import { selectText } from './page/select-text';
import { setChecked } from './page/set-checked';
import { siteToolResult } from './page/site-tool-result';
import { snapshot } from './page/snapshot';
import { storage } from './page/storage';
import { solveCaptcha } from './page/solve-captcha';
import { startDiagnostics } from './page/start-diagnostics';
import { startMonitor } from './page/start-monitor';
import { startTimer } from './page/start-timer';
import { stopDiagnostics } from './page/stop-diagnostics';
import { stopMonitor } from './page/stop-monitor';
import { stopTimer } from './page/stop-timer';
import { submitForm } from './page/submit-form';
import { switchFrame } from './page/switch-frame';
import { switchTab } from './page/switch-tab';
import { timerStatus } from './page/timer-status';
import { trustedClick } from './page/trusted-click';
import { typeText } from './page/type-text';
import { waitFor } from './page/wait-for';
import { waitForElement } from './page/wait-for-element';

export const actions: ReadonlyMap<string, AnyAction> = new Map(
  (
    [
      getPageInfo,
      snapshot,
      readHtml,
      scrollTo,
      clickElement,
      trustedClick,
      findCaptcha,
      solveCaptcha,
      hoverElement,
      dragElement,
      focusInput,
      fillInput,
      typeText,
      selectOption,
      setChecked,
      selectText,
      extractText,
      query,
      pickElement,
      pressKey,
      submitForm,
      waitForElement,
      waitFor,
      insertText,
      mouse,
      evaluate,
      initScript,
      storage,
      cookies,
      clipboard,
      injectCode,
      runCode,
      listSiteTools,
      callSiteTool,
      siteToolResult,
      cancelSiteTool,
      checkSites,
      findProgress,
      findSearch,
      readTheme,
      auditContrast,
      applyTheme,
      startDiagnostics,
      readConsole,
      readNetwork,
      stopDiagnostics,
      routeNetwork,
      unrouteNetwork,
      harStart,
      harStop,
      dialog,
      emulate,
      pdf,
      recordStart,
      recordStop,
      traceStart,
      traceStop,
      vitals,
      react,
      a11yAudit,
      startMonitor,
      monitorStatus,
      awaitMonitor,
      stopMonitor,
      startTimer,
      timerStatus,
      stopTimer,
      highlightElement,
      searchSite,
      navigate,
      openTab,
      openWindow,
      resizeWindow,
      pushState,
      tabKeep,
      switchTab,
      switchFrame,
      closeTab,
      screenshot,
      listFiles,
      attachFile,
      captureDownload,
      listDownloads,
      listRecordings,
      readRecording,
    ] as AnyAction[]
  ).map((action) => [action.name, action]),
);

export type BrowserTarget = 'chromium' | 'firefox';

/**
 * The tools a build of the extension offers. Every action stays invocable on either browser —
 * a stale skill that names a hidden one gets its UNSUPPORTED hint, not UNKNOWN_ACTION — but a
 * Firefox build does not list what it can never run.
 */
export function describeActions(target: BrowserTarget = 'chromium'): ToolDescriptor[] {
  return [...actions.values()]
    .filter((action) => target === 'chromium' || !action.chromiumOnly)
    .map(({ name, description, input }) => ({
      name,
      description,
      inputSchema: tidy(z.toJSONSchema(input, { io: 'input' })),
    }));
}

const UNBOUNDED = new Set([Number.MAX_SAFE_INTEGER, Number.MIN_SAFE_INTEGER]);

/**
 * Every tool's schema is re-sent to the model on every turn, so what zod emits for its
 * own sake — the dialect URL, and the safe-integer bounds `.int()` implies — is cut, and
 * so are the fields only the Vexdom Bridge fills in, such as a target's resolved `ref`.
 */
function tidy(schema: unknown): unknown {
  if (Array.isArray(schema)) return schema.map(tidy);
  if (!schema || typeof schema !== 'object') return schema;
  return Object.fromEntries(
    Object.entries(schema)
      .filter(
        ([key, value]) =>
          key !== '$schema' && !(isBound(key) && UNBOUNDED.has(value as number)) && !isUndescribed(value),
      )
      .map(([key, value]) => [key, tidy(value)]),
  );
}

const isBound = (key: string) => key === 'minimum' || key === 'maximum';

const isUndescribed = (value: unknown) =>
  !!value && typeof value === 'object' && (value as Record<string, unknown>)[UNDESCRIBED] === true;
