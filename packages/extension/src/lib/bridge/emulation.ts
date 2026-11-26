/**
 * `page.emulate`: another screen, phone, place, network or colour scheme for one tab, with the
 * CDP calls agent-browser makes (`handle_viewport`, `handle_device`, `handle_geolocation`,
 * `handle_offline`, `handle_headers`, `handle_set_media`). Headers and credentials share one
 * `Network.setExtraHTTPHeaders` call, so setting one keeps the other. The `emulation` lease is
 * held until `reset`.
 */
import { z } from 'zod';
import { emulate } from '@/lib/actions/page/emulate';
import { failure, success, type ActionResult } from '@/lib/actions/protocol';
import { acquire, detachedFailure, onLost, release, send } from './cdp';
import { DEVICES, devicePreset } from './devices';

const OWNER = 'emulation';
const FIREFOX_HINT = 'Emulation needs Chrome’s debugger, which Firefox does not offer to extensions.';

interface Emulated {
  headers: Record<string, string>;
  authorization?: string;
  originalUserAgent?: string;
}

const tabs = new Map<number, Emulated>();
const lost = new Map<number, string>();

type Input = z.output<typeof emulate.input>;

export async function emulateInTab(tabId: number, input: unknown): Promise<ActionResult> {
  const parsed = emulate.input.safeParse(input ?? {});
  if (!parsed.success) return failure('INVALID_INPUT', z.prettifyError(parsed.error));
  const asked: Input = parsed.data;
  if (asked.reset) return resetTab(tabId);
  const { viewport, device, geolocation, offline, headers, credentials, media } = asked;
  if ([viewport, device, geolocation, offline, headers, credentials, media].every((section) => section === undefined)) {
    return failure('INVALID_INPUT', 'Give at least one of viewport, device, geolocation, offline, headers, credentials, media or reset.');
  }

  // An emulation that went with the debugger is not restored: this call starts afresh.
  lost.delete(tabId);
  const acquired = await acquire(tabId, OWNER, FIREFOX_HINT);
  if (!acquired.ok) return failure(acquired.code, acquired.message);
  const session = acquired.lease.session;
  const state = tabs.get(tabId) ?? { headers: {} };
  tabs.set(tabId, state);
  const applied: Record<string, unknown> = {};
  try {
    if (asked.device !== undefined) {
      const preset = devicePreset(asked.device);
      if (!preset) {
        return failure('UNKNOWN_DEVICE', `Unknown device: ${asked.device}. Supported: ${DEVICES.map((device) => device.name).join(', ')}`);
      }
      await setMetrics(session, preset.width, preset.height, preset.deviceScaleFactor, preset.mobile);
      await setUserAgent(tabId, session, state, preset.userAgent);
      applied.device = { name: preset.name, width: preset.width, height: preset.height, deviceScaleFactor: preset.deviceScaleFactor, mobile: preset.mobile };
    }
    if (asked.viewport) {
      const { width, height, deviceScaleFactor, mobile } = asked.viewport;
      await setMetrics(session, width, height, deviceScaleFactor, mobile);
      applied.viewport = asked.viewport;
    }
    if (asked.geolocation) {
      await send(session, 'Emulation.setGeolocationOverride', asked.geolocation);
      applied.geolocation = asked.geolocation;
    }
    if (asked.offline !== undefined) {
      await send(session, 'Network.emulateNetworkConditions', {
        offline: asked.offline,
        latency: 0,
        downloadThroughput: -1,
        uploadThroughput: -1,
      });
      applied.offline = asked.offline;
    }
    if (asked.headers || asked.credentials) {
      if (asked.headers) state.headers = asked.headers;
      if (asked.credentials) {
        state.authorization = `Basic ${btoa(`${asked.credentials.username}:${asked.credentials.password}`)}`;
      }
      await send(session, 'Network.enable');
      await send(session, 'Network.setExtraHTTPHeaders', {
        headers: { ...state.headers, ...(state.authorization ? { Authorization: state.authorization } : {}) },
      });
      if (asked.headers) applied.headers = asked.headers;
      if (asked.credentials) applied.credentials = { username: asked.credentials.username };
    }
    if (asked.media) {
      const features: { name: string; value: string }[] = [];
      if (asked.media.colorScheme) features.push({ name: 'prefers-color-scheme', value: asked.media.colorScheme });
      if (asked.media.reducedMotion) features.push({ name: 'prefers-reduced-motion', value: asked.media.reducedMotion });
      await send(session, 'Emulation.setEmulatedMedia', {
        ...(asked.media.type ? { media: asked.media.type } : {}),
        ...(features.length ? { features } : {}),
      });
      applied.media = asked.media;
    }
  } catch (error) {
    return failure('ACTION_FAILED', error instanceof Error ? error.message : String(error));
  }
  return success(applied);
}

async function setMetrics(session: { tabId: number }, width: number, height: number, deviceScaleFactor: number, mobile: boolean) {
  await send(session, 'Emulation.setDeviceMetricsOverride', { width, height, deviceScaleFactor, mobile });
  await send(session, 'Emulation.setTouchEmulationEnabled', { enabled: mobile });
}

async function setUserAgent(tabId: number, session: { tabId: number }, state: Emulated, userAgent: string) {
  state.originalUserAgent ??= globalThis.navigator?.userAgent;
  await send(session, 'Network.setUserAgentOverride', { userAgent });
  tabs.set(tabId, state);
}

async function resetTab(tabId: number): Promise<ActionResult> {
  const state = tabs.get(tabId);
  const gone = lost.get(tabId);
  lost.delete(tabId);
  if (!state) return gone ? detachedFailure(gone) : success({ reset: true });
  const session = { tabId };
  const quietly = (method: string, params?: Record<string, unknown>) => send(session, method, params).catch(() => undefined);
  await quietly('Emulation.clearDeviceMetricsOverride');
  await quietly('Emulation.clearGeolocationOverride');
  await quietly('Emulation.setEmulatedMedia', {});
  await quietly('Network.emulateNetworkConditions', { offline: false, latency: 0, downloadThroughput: -1, uploadThroughput: -1 });
  await quietly('Network.setExtraHTTPHeaders', { headers: {} });
  if (state.originalUserAgent) await quietly('Network.setUserAgentOverride', { userAgent: state.originalUserAgent });
  await quietly('Emulation.setTouchEmulationEnabled', { enabled: false });
  tabs.delete(tabId);
  await release(tabId, OWNER);
  return success({ reset: true });
}

export function serveEmulation(): void {
  onLost((tabId, reason) => {
    if (tabs.delete(tabId)) lost.set(tabId, reason);
  });
}
