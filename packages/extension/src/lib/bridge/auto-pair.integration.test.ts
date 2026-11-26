import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, test, vi } from 'vitest';
import { fakeBrowser } from 'wxt/testing';
import { EXT_CORE_NODE, startRustBridge, type RustBridge } from './test/rust-bridge';
import type { DaemonState } from './socket';

const ports = vi.hoisted(() => [] as number[]);
vi.mock('@/lib/actions/protocol', async (importOriginal) => ({
  ...(await importOriginal<object>()),
  DAEMON_PORTS: ports,
}));

// ext-core's node build stands in for the wasm the browser fetches.
vi.mock('./core', async () => {
  const glue = (await import(/* @vite-ignore */ EXT_CORE_NODE)) as { initialized: Promise<void> };
  await glue.initialized;
  return { ...glue, loadCore: async () => undefined };
});

const ORIGIN = 'chrome-extension://abcdefghijklmnopabcdefghijklmnop';

type NodeWebSocket = new (url: string, init: { headers: Record<string, string> }) => WebSocket;

class ExtensionSocket extends (WebSocket as unknown as NodeWebSocket) {
  constructor(url: string) {
    super(url, { headers: { origin: ORIGIN } });
  }
}
vi.stubGlobal('WebSocket', ExtensionSocket);

type Socket = typeof import('./socket');
type AutoPair = typeof import('./auto-pair');

let bridge: RustBridge;
let socket: Socket | undefined;
let pairing: AutoPair;
let host: ReturnType<typeof vi.fn>;
const TEST_TIMEOUT_MS = 20_000;

/** The real host's answer, as the browser gets it with Chrome passing this extension's origin. */
const nativeHost = (message: unknown) => bridge.nativeHost(message, `${ORIGIN}/`);

beforeAll(() => {
  bridge = startRustBridge();
  ports.push(bridge.port);
});

afterAll(() => bridge?.stop());

beforeEach(async () => {
  fakeBrowser.reset();
  vi.resetModules();
  vi.spyOn(fakeBrowser.runtime, 'getManifest').mockReturnValue({ version: '0.0.0-test' } as never);
  host = vi.fn((_name: string, message: unknown) => nativeHost(message));
  (fakeBrowser.runtime as unknown as { sendNativeMessage: typeof host }).sendNativeMessage = host;
  socket = await import('./socket');
  pairing = await import('./auto-pair');
  pairing.serveAutoPair();
});

afterEach(async () => {
  await socket?.disconnectDaemon();
  await new Promise((resolve) => setTimeout(resolve, 50));
  bridge.forgetSessions();
  bridge.resetConfig();
});

const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

async function daemonState(): Promise<DaemonState | undefined> {
  return (await fakeBrowser.storage.session.get('vexdom/daemon'))['vexdom/daemon'] as DaemonState | undefined;
}

async function until(check: (state: DaemonState | undefined) => boolean, what: string): Promise<void> {
  for (let tries = 0; !check(await daemonState()); tries++) {
    if (tries > 250) throw new Error(`never ${what}: ${JSON.stringify(await daemonState())}`);
    await wait(20);
  }
}

const online = () => until((state) => !!state?.connected, 'came online');

const pairCode = () => /Pairing code:\s+([A-Z0-9-]+)/.exec(bridge.cli('pair'))![1].replace('-', '');

const pairRequests = () => (JSON.parse(bridge.cli('requests', '--json')) as { requests: { id: string }[] }).requests;

describe('pairing by itself', () => {
  test('a fresh install pairs with no code', async () => {
    await pairing.autoPair('install');
    await online();
    expect(bridge.sessions()).toEqual([expect.objectContaining({ pairedBy: 'auto', origin: ORIGIN })]);
  }, TEST_TIMEOUT_MS);

  test('a browser that holds a key never asks the host', async () => {
    expect(await socket!.pairDaemon(pairCode())).toEqual({ ok: true });

    await pairing.autoPair('alarm');
    expect(host).not.toHaveBeenCalled();
  }, TEST_TIMEOUT_MS);

  test('with no Bridge, says so and looks again a minute later, not at every alarm', async () => {
    host.mockRejectedValue(new Error('Specified native messaging host not found.'));
    await pairing.autoPair('install');
    await until((state) => state?.bridge === 'missing', 'said the Bridge is missing');

    await pairing.autoPair('alarm');
    expect(host).toHaveBeenCalledOnce();
  }, TEST_TIMEOUT_MS);

  test('a click looks again at once', async () => {
    host.mockRejectedValueOnce(new Error('Specified native messaging host not found.'));
    await pairing.autoPair('install');
    await pairing.autoPair('gesture', { gesture: true });
    await online();
  }, TEST_TIMEOUT_MS);

  test('a browser start during an alarm that was not due still gets its own look', async () => {
    await fakeBrowser.storage.local.set({ 'vexdom/pairLast': { at: Date.now() - 2 * 60_000, outcome: 'stopped', failures: 0 } });

    await Promise.all([pairing.autoPair('alarm'), pairing.autoPair('startup')]);

    expect(host).toHaveBeenCalledOnce();
    await online();
  }, TEST_TIMEOUT_MS);

  test('looks for a missing Bridge every minute again once the browser starts again', async () => {
    host.mockRejectedValue(new Error('Specified native messaging host not found.'));
    const longAgo = Date.now() - 60 * 60_000;
    await fakeBrowser.storage.local.set({ 'vexdom/pairLast': { at: Date.now() - 6_000, outcome: 'missing', missingSince: longAgo, failures: 0 } });

    await pairing.autoPair('startup');

    const { 'vexdom/pairLast': last } = await fakeBrowser.storage.local.get('vexdom/pairLast');
    expect((last as { missingSince: number }).missingSince).toBeGreaterThan(longAgo);
  }, TEST_TIMEOUT_MS);

  test('keeps the key a typed code gave it while the host was answering, rather than redeeming the grant', async () => {
    host.mockImplementationOnce(async (_name: string, message: unknown) => {
      expect(await socket!.pairDaemon(pairCode())).toEqual({ ok: true });
      return nativeHost(message);
    });

    await pairing.autoPair('install');

    expect(bridge.sessions()).toEqual([expect.objectContaining({ pairedBy: 'code' })]);
  }, TEST_TIMEOUT_MS);

  test('says when automatic pairing is off, and leaves the code to the user', async () => {
    bridge.cli('pairing', 'code');

    await pairing.autoPair('install');
    await until((state) => state?.bridge === 'off', 'said pairing is off');
    expect(bridge.sessions()).toEqual([]);
  }, TEST_TIMEOUT_MS);
});

describe('staying unpaired when asked to', () => {
  test('Disconnect holds it off until Connect', async () => {
    await pairing.autoPair('install');
    await online();
    await socket!.disconnectDaemon();
    await pairing.holdAutoPair();
    host.mockClear();

    await pairing.autoPair('alarm');
    expect(host).not.toHaveBeenCalled();
    expect((await daemonState())?.pairing).toEqual({ manual: true });

    await pairing.autoPair('gesture', { gesture: true });
    await online();
  }, TEST_TIMEOUT_MS);

  test('a browser this computer unpaired is told so, and comes back only on Reconnect', async () => {
    await pairing.autoPair('install');
    await online();
    bridge.cli('revoke', bridge.sessions()[0].installId!);
    await until((state) => !state?.connected, 'went offline');

    await socket!.connectDaemon();
    await until((state) => state?.pairing?.blocked === 'revoked', 'heard it was revoked');
    host.mockClear();
    await pairing.autoPair('alarm');
    expect(host).not.toHaveBeenCalled();

    await pairing.autoPair('gesture', { gesture: true });
    await online();
  }, TEST_TIMEOUT_MS);
});

describe('a Bridge that forgot this browser', () => {
  test('is paired with again, and the old key is kept until the new one arrives', async () => {
    await pairing.autoPair('install');
    await online();
    const { 'vexdom/sessionKey': before } = await fakeBrowser.storage.local.get('vexdom/sessionKey');
    await socket!.disconnectDaemon();
    await fakeBrowser.storage.local.set({ 'vexdom/sessionKey': before });
    await fakeBrowser.storage.session.set({ 'vexdom/daemon': { connected: false, paired: true, lastChangeAt: Date.now() } });
    bridge.forgetSessions();

    await socket!.connectDaemon();
    await online();
    const { 'vexdom/sessionKey': after } = await fakeBrowser.storage.local.get('vexdom/sessionKey');
    expect(after).toBeTypeOf('string');
    expect(after).not.toBe(before);
  }, TEST_TIMEOUT_MS);
});

describe('ask mode', () => {
  test('waits for someone to allow this browser, and connects the moment they do', async () => {
    bridge.cli('pairing', 'ask');

    const asking = pairing.autoPair('open', { wait: true });
    let requests = pairRequests();
    while (!requests.length) {
      await wait(20);
      requests = pairRequests();
    }
    bridge.cli('approve', requests[0].id);

    await asking;
    await online();
    expect(bridge.sessions()).toEqual([expect.objectContaining({ pairedBy: 'approved' })]);
  }, TEST_TIMEOUT_MS);

  test('shows who has to allow it while nobody has', async () => {
    bridge.cli('pairing', 'ask');

    await pairing.autoPair('open');
    await until((state) => !!state?.pairing?.wait, 'said it is waiting');
    expect((await daemonState())?.pairing).toMatchObject({ mode: 'ask', wait: { approver: 'terminal' } });
  }, TEST_TIMEOUT_MS);
});

describe('isDue', () => {
  const at = 1_000_000_000;
  const minute = 60_000;

  test('spaces attempts by what the last one found', () => {
    const last = (outcome: Parameters<typeof pairing.isDue>[1] extends infer L ? (L extends { outcome: infer O } ? O : never) : never, ago: number, extra = {}) => ({
      at: at - ago,
      outcome,
      failures: 0,
      ...extra,
    });
    expect(pairing.isDue('alarm', undefined, false, at)).toBe(true);
    expect(pairing.isDue('open', last('missing', 1_000), false, at)).toBe(false);
    expect(pairing.isDue('open', last('missing', 6_000), false, at)).toBe(true);
    expect(pairing.isDue('alarm', last('missing', minute, { missingSince: at - 2 * minute }), false, at)).toBe(true);
    expect(pairing.isDue('alarm', last('missing', 2 * minute, { missingSince: at - 20 * minute }), false, at)).toBe(false);
    expect(pairing.isDue('alarm', last('old', 30 * minute), false, at)).toBe(false);
    expect(pairing.isDue('alarm', last('failed', 3 * minute, { failures: 3 }), false, at)).toBe(false);
    expect(pairing.isDue('alarm', last('failed', 5 * minute, { failures: 3 }), false, at)).toBe(true);
    expect(pairing.isDue('alarm', last('off', 600 * minute), false, at)).toBe(false);
    expect(pairing.isDue('startup', last('off', 6_000), false, at)).toBe(true);
  });

  test('never tries a revoked or denied browser again unasked, and always on a click', () => {
    for (const outcome of ['revoked', 'denied'] as const) {
      const last = { at: at - 600 * minute, outcome, failures: 0 };
      expect(['alarm', 'open', 'startup', 'install'].some((reason) => pairing.isDue(reason as 'alarm', last, false, at))).toBe(false);
      expect(pairing.isDue('gesture', last, true, at)).toBe(true);
    }
  });
});
