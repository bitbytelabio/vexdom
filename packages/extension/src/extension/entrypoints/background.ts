import '@/lib/zod-locale';
import { failure, isBridgeRequest, success } from '@/lib/actions/protocol';
import { refusalFor } from '@/lib/bridge/site-guard';
import { LIST_UNREADABLE_MESSAGE, SITE_BLOCKED } from '@/lib/settings/blocked-sites';
import { injectContentScript } from '@/lib/actions/client';
import { describeOwnActions } from '@/lib/bridge/own-actions';
import { invokeForHarness } from '@/lib/bridge/invoke';
import { serveDebuggerEvents } from '@/lib/bridge/cdp';
import { serveAutoRuns } from '@/lib/bridge/auto-run';
import { serveCodeToolkits } from '@/lib/bridge/code-toolkit';
import { serveDiagnostics } from '@/lib/bridge/diagnostics';
import { serveFrameFocus } from '@/lib/bridge/frame-focus';
import { serveTabLabels } from '@/lib/bridge/tab-labels';
import { serveNetworkRoutes } from '@/lib/bridge/network-route';
import { serveHar } from '@/lib/bridge/har';
import { serveDialogs } from '@/lib/bridge/dialogs';
import { serveEmulation } from '@/lib/bridge/emulation';
import { serveInitScripts } from '@/lib/bridge/init-scripts';
import { serveTabKeep } from '@/lib/bridge/tab-keep';
import { serveTrustedInput } from '@/lib/bridge/trusted-input';
import { serveCapture } from '@/lib/bridge/capture';
import { serveDevtools } from '@/lib/bridge/devtools';
import { resumePendingFiles } from '@/lib/bridge/file-store';
import { pushSkill, removeSkill, resyncSkills } from '@/lib/bridge/skill-store';
import { nameStoredSession } from '@/lib/bridge/session-store';
import { analyzeStoredRecording, resumePendingAnalyses } from '@/lib/bridge/recording-store';
import { ingestSample, monitorsForTab, serveMonitor } from '@/lib/bridge/monitor';
import { appendEvents, recordingStateFor, serveRecorder } from '@/lib/bridge/recorder';
import { onPanelPresence, runCommand, serveRunPorts, serveTabSessions, sweepFiles } from '@/lib/bridge/run-port';
import { serveRunIndicator } from '@/lib/bridge/run-indicator';
import { sessionForTab } from '@/lib/bridge/tab-sessions';
import { HAND_OFF_ACTION } from '@/lib/actions/reserved';
import { clearStrandedRail, serveRail, setPanelCollapsed, syncRail } from '@/lib/bridge/rail';
import { serveTabGroups } from '@/lib/bridge/tab-groups';
import { serveSeenSites } from '@/lib/bridge/seen-sites';
import { captureRegion } from '@/lib/bridge/region';
import { serveTimers } from '@/lib/bridge/timer';
import { serveTaskNotices } from '@/lib/bridge/task-notices';
import { serveRunNotices } from '@/lib/bridge/run-notices';
import { serveToast } from '@/lib/bridge/toast';
import { serveHandsFree } from '@/lib/bridge/hands-free';
import { endHandsFree } from '@/lib/bridge/panel-view';
import { serveLaunchers } from '@/lib/bridge/launchers';
import { servePreferences } from '@/lib/bridge/preferences';
import { serveExtensionUpdates } from '@/lib/bridge/extension-update';
import { autoPair, holdAutoPair, releaseAutoPair, serveAutoPair } from '@/lib/bridge/auto-pair';
import { serveDeviceNotices } from '@/lib/bridge/device-notices';
import { isAgentKind } from '@/lib/agents/catalog';
import { isPreferenceChange } from '@/lib/settings/preferences';
import {
  RECONNECT_ALARM,
  chooseAgent,
  chooseAgentModel,
  connectDaemon,
  retryAfterUpdate,
  disconnectDaemon,
  forgetApprovals,
  onWelcome,
  readApprovals,
  pairDaemon,
  readAgentState,
  readPreferences,
  reportFocus,
  readSkillCatalog,
  setPreference,
  setUpAgent,
  taskOp,
} from '@/lib/bridge/socket';
import { loadCore } from '@/lib/bridge/pure';

export default defineBackground(() => {
  serveDebuggerEvents();

  browser.runtime.onMessage.addListener((message, sender, sendResponse) => {
    if (!isBridgeRequest(message)) return;
    if (message.op === 'describe') {
      sendResponse(success(describeOwnActions()));
      return;
    }
    if (message.op === 'invoke' && typeof message.action === 'string') {
      invokeForHarness(message.action, message.input)
        .then(sendResponse)
        .catch((error) => sendResponse(failure('BRIDGE_ERROR', String(error))));
      return true;
    }
    if (message.op === 'saveSkill' && typeof message.skillId === 'string') {
      void pushSkill(message.skillId);
      sendResponse(success(true));
      return;
    }
    if (message.op === 'removeSkill' && typeof message.skillId === 'string') {
      void removeSkill(message.skillId);
      sendResponse(success(true));
      return;
    }
    if (message.op === 'nameSession' && typeof message.sessionId === 'string') {
      void nameStoredSession(message.sessionId);
      sendResponse(success(true));
      return;
    }
    if (message.op === 'recordEvents' && Array.isArray(message.events)) {
      void appendEvents(sender.tab?.id, message.events, sender.url);
      sendResponse(success(true));
      return;
    }
    if (message.op === 'recordingState') {
      recordingStateFor(sender.tab?.id, sender.url)
        .then((state) => sendResponse(success(state)))
        .catch(() => sendResponse(success({ recording: false, captureValues: false })));
      return true;
    }
    if (message.op === 'analyzeRecording' && typeof message.recordingId === 'string') {
      void analyzeStoredRecording(message.recordingId);
      sendResponse(success(true));
      return;
    }
    if (message.op === 'monitorSample' && typeof message.monitorId === 'string') {
      void ingestSample(sender.tab?.id, message.monitorId, message.sample, sender.url);
      sendResponse(success(true));
      return;
    }
    if (message.op === 'siteRefusal' && typeof message.url === 'string') {
      refusalFor(message.url)
        .then((refused) => sendResponse(refused))
        .catch(() => sendResponse(failure(SITE_BLOCKED, LIST_UNREADABLE_MESSAGE)));
      return true;
    }
    if (message.op === 'monitorState') {
      monitorsForTab(sender.tab?.id, sender.url)
        .then(sendResponse)
        .catch(() => sendResponse(success({ monitors: [] })));
      return true;
    }
    if (message.op === 'listSkills') {
      readSkillCatalog(message.refresh === true)
        .then(sendResponse)
        .catch((error) => sendResponse(failure('BRIDGE_ERROR', String(error))));
      return true;
    }
    if (message.op === 'agentState') {
      readAgentState(message.refresh === true)
        .then(sendResponse)
        .catch((error) => sendResponse(failure('BRIDGE_ERROR', String(error))));
      return true;
    }
    if (message.op === 'setAgent' && isAgentKind(message.agent)) {
      chooseAgent(message.agent)
        .then(sendResponse)
        .catch((error) => sendResponse(failure('BRIDGE_ERROR', String(error))));
      return true;
    }
    if (message.op === 'setAgentModel' && isAgentKind(message.agent)) {
      chooseAgentModel(message.agent, typeof message.model === 'string' && message.model.trim() ? message.model : null)
        .then(sendResponse)
        .catch((error) => sendResponse(failure('BRIDGE_ERROR', String(error))));
      return true;
    }
    if (message.op === 'grantAgent' && isAgentKind(message.agent)) {
      setUpAgent(message.agent)
        .then(sendResponse)
        .catch((error) => sendResponse(failure('BRIDGE_ERROR', String(error))));
      return true;
    }
    if (message.op === 'preferences') {
      readPreferences()
        .then(sendResponse)
        .catch((error) => sendResponse(failure('BRIDGE_ERROR', String(error))));
      return true;
    }
    if (message.op === 'setPreference') {
      // isPreferenceChange checks a profile in ext-core, which may not be loaded yet.
      loadCore()
        .then(() =>
          isPreferenceChange(message.change)
            ? setPreference(message.change)
            : failure('INVALID_INPUT', 'Not a setting this page can change.'),
        )
        .then(sendResponse)
        .catch((error) => sendResponse(failure('BRIDGE_ERROR', String(error))));
      return true;
    }
    if (message.op === 'tasks' || message.op === 'pauseTasks') {
      taskOp(message.op === 'tasks' ? { t: 'tasks' } : { t: 'pauseTasks', paused: message.paused === true })
        .then(sendResponse)
        .catch((error) => sendResponse(failure('BRIDGE_ERROR', String(error))));
      return true;
    }
    if (message.op === 'saveTask') {
      taskOp({ t: 'saveTask', task: message.task })
        .then(sendResponse)
        .catch((error) => sendResponse(failure('BRIDGE_ERROR', String(error))));
      return true;
    }
    if ((message.op === 'deleteTask' || message.op === 'runTaskNow') && typeof message.taskId === 'string') {
      taskOp({ t: message.op, taskId: message.taskId })
        .then(sendResponse)
        .catch((error) => sendResponse(failure('BRIDGE_ERROR', String(error))));
      return true;
    }
    if (message.op === 'pair' && typeof message.token === 'string') {
      pairDaemon(message.token)
        .then(async (result) => {
          if (result.ok) await releaseAutoPair();
          sendResponse(result.ok ? success(true) : failure('PAIRING_FAILED', result.error ?? 'Pairing failed'));
        })
        .catch((error) => sendResponse(failure('BRIDGE_ERROR', String(error))));
      return true;
    }
    if (message.op === 'panelOpened') {
      void setPanelCollapsed(false);
      void endHandsFree();
      sendResponse(success(true));
      return;
    }
    if (message.op === 'disconnect') {
      disconnectDaemon()
        .then(holdAutoPair)
        .then(() => sendResponse(success(true)))
        .catch((error) => sendResponse(failure('BRIDGE_ERROR', String(error))));
      return true;
    }
    if (message.op === 'autoPair') {
      const gesture = message.gesture === true;
      autoPair(gesture ? 'gesture' : 'open', { gesture, wait: message.wait === true })
        .then(() => sendResponse(success(true)))
        .catch((error) => sendResponse(failure('BRIDGE_ERROR', String(error))));
      return true;
    }
    if (message.op === 'approvals' || message.op === 'forgetApprovals') {
      const host =
        message.op === 'forgetApprovals' && typeof message.host === 'string' && message.host ? message.host : undefined;
      (message.op === 'approvals' ? readApprovals() : forgetApprovals(host))
        .then(sendResponse)
        .catch((error) => sendResponse(failure('BRIDGE_ERROR', String(error))));
      return true;
    }
    if (message.op === 'captureRegion' && typeof message.tabId === 'number') {
      captureRegion(message.tabId)
        .then(sendResponse)
        .catch((error) => sendResponse(failure('BRIDGE_ERROR', String(error))));
      return true;
    }
    if (message.op === 'stopRun' || message.op === 'handBack') {
      const tabId = sender.tab?.id;
      const handBack = message.op === 'handBack' ? message.allow === true : null;
      void (tabId != null ? sessionForTab(tabId) : Promise.resolve(null)).then((session) => {
        if (!session?.runId) return;
        if (handBack === null) runCommand({ op: 'cancel', sessionId: session.sessionId });
        else if (session.pendingApproval?.action === HAND_OFF_ACTION) {
          runCommand({
            op: 'decision',
            sessionId: session.sessionId,
            toolId: session.pendingApproval.toolId,
            allow: handBack,
          });
        }
      });
      sendResponse(success(true));
      return;
    }
    sendResponse(
      failure(
        'INVALID_REQUEST',
        'Expected {op:"describe"|"invoke"|"saveSkill"|"removeSkill"|"nameSession"|"recordEvents"|"recordingState"|"analyzeRecording"|"monitorSample"|"monitorState"|"listSkills"|"agentState"|"setAgent"|"setAgentModel"|"grantAgent"|"preferences"|"setPreference"|"tasks"|"saveTask"|"deleteTask"|"runTaskNow"|"pauseTasks"|"pair"|"autoPair"|"panelOpened"|"disconnect"|"stopRun"|"handBack"|"captureRegion"|"approvals"|"forgetApprovals"}',
      ),
    );
    return;
  });

  browser.runtime.onInstalled.addListener(({ reason }) => {
    void browser.tabs.query({}).then((tabs) => {
      for (const tab of tabs) {
        if (tab.id != null && !tab.discarded) void injectContentScript(tab.id);
      }
    });
    if (reason === 'install') {
      void browser.tabs.create({ url: browser.runtime.getURL('/welcome.html') });
      void autoPair('install');
    }
    if (reason === 'update') void retryAfterUpdate().then(() => autoPair('update'));
  });

  onPanelPresence((open) => {
    if (!open) void clearStrandedRail();
  });

  serveLaunchers();
  serveDiagnostics();
  serveFrameFocus();
  serveTabLabels();
  serveNetworkRoutes();
  serveHar();
  serveDialogs();
  serveEmulation();
  serveInitScripts();
  serveTabKeep();
  serveTrustedInput();
  serveCapture();
  serveDevtools();
  serveCodeToolkits();
  serveAutoRuns();
  serveRecorder();
  serveMonitor();
  serveTimers();
  serveTaskNotices();
  serveRunNotices();
  serveRunIndicator();
  serveRunPorts();
  serveTabSessions();
  serveTabGroups();
  serveSeenSites();
  serveRail();
  serveToast();
  serveHandsFree();
  servePreferences();
  serveExtensionUpdates();
  void syncRail();

  onWelcome(() => {
    void resyncSkills();
    void resumePendingAnalyses();
    void resumePendingFiles();
  });
  void sweepFiles();

  serveAutoPair();
  serveDeviceNotices();
  void connectDaemon().then(() => autoPair('alarm'));
  browser.runtime.onStartup.addListener(() => void connectDaemon().then(() => autoPair('startup')));

  // With several browsers connected, a caller outside any of them reaches the one last in front.
  browser.windows?.onFocusChanged.addListener((windowId) => {
    if (windowId !== browser.windows.WINDOW_ID_NONE) reportFocus();
  });

  browser.alarms.create(RECONNECT_ALARM, { periodInMinutes: 1 });
  browser.alarms.onAlarm.addListener((alarm) => {
    if (alarm.name === RECONNECT_ALARM) void connectDaemon().then(() => autoPair('alarm'));
  });
});
