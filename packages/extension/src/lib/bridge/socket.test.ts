import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest';
import { fakeBrowser } from 'wxt/testing';
import { serverProof, type Transcript } from '@/lib/actions/handshake';
import type { LinkEvent } from './core';

const SESSION_KEY = 'session-key-of-a-paired-browser';

/** ext-core as socket.ts sees it: the Bridge's side is played here, one event at a time. */
const core = vi.hoisted(() => ({
  events: [] as unknown[],
  transcript: undefined as Transcript | undefined,
  replies: [] as [number, unknown][],
  shots: [] as [number, unknown, Uint8Array][],
  socket: undefined as { onmessage: ((event: { data: ArrayBuffer }) => void) | null } | undefined,
  /** An unproven listener: it answers the hello and never welcomes. */
  withholdWelcome: false,
}));

/** Lets socket.ts pump: what a frame from the Bridge does. */
const arrive = (...events: LinkEvent[]) => {
  core.events.push(...events);
  core.socket?.onmessage?.({ data: new ArrayBuffer(0) });
};

vi.mock('./core', () => ({
  loadCore: async () => undefined,
  linkOpen: () => undefined,
  linkFeed: () => undefined,
  linkClose: () => undefined,
  linkPump: () => new Uint8Array(),
  linkEvents: () => core.events.splice(0),
  linkHello: (hello: { protocolVersion: number; extensionVersion: string; manifestHash: string; nonce: string }) => {
    core.transcript = {
      protocolVersion: hello.protocolVersion,
      extensionVersion: hello.extensionVersion,
      manifestHash: hello.manifestHash,
      clientNonce: hello.nonce,
      serverNonce: 'A'.repeat(43),
    };
    core.events.push({ tag: 'ServerNonce', nonce: core.transcript.serverNonce });
  },
  linkProve: () => {
    if (core.withholdWelcome) return;
    const claims = { daemonVersion: '0.9.2', manifestHash: core.transcript!.manifestHash, manifestInSync: true, sealedSessionKey: '' };
    void serverProof(SESSION_KEY, core.transcript!, claims).then((proof) =>
      arrive({ tag: 'Welcome', ...claims, protocolVersion: 27, proof }),
    );
  },
  linkRequest: () => 1,
  linkTrust: () => undefined,
  linkReply: (callId: number, result: string) => core.replies.push([callId, JSON.parse(result)]),
  linkReplyScreenshot: (callId: number, result: string, image: Uint8Array) => core.shots.push([callId, JSON.parse(result), image]),
  linkReplyArtifact: (callId: number, result: string, data: Uint8Array) => core.shots.push([callId, JSON.parse(result), data]),
}));

vi.mock('./invoke', () => ({
  invokeForHarness: vi.fn(async (action: string, input: unknown, tabId?: number) =>
    action === 'page.screenshot'
      ? { ok: true, data: { format: 'png', width: 1, height: 1, dataUrl: 'data:image/png;base64,AQID' } }
      : { ok: true, data: { action, input, tabId } },
  ),
}));

vi.mock('@/lib/actions/protocol', async (importOriginal) => ({
  ...(await importOriginal<object>()),
  DAEMON_PORTS: [8785],
}));

class FakeSocket {
  static readonly OPEN = 1;
  readyState = 1;
  binaryType = 'blob';
  onopen: (() => void) | null = null;
  onmessage: ((event: { data: ArrayBuffer }) => void) | null = null;
  onclose: ((event: { reason: string }) => void) | null = null;
  constructor() {
    core.socket = this;
    setTimeout(() => this.onopen?.());
  }
  send(): void {}
  close(): void {
    this.readyState = 3;
  }
}
vi.stubGlobal('WebSocket', FakeSocket);

let worker: typeof import('./socket') | undefined;

beforeEach(async () => {
  fakeBrowser.reset();
  vi.resetModules();
  core.events.length = core.replies.length = core.shots.length = 0;
  core.withholdWelcome = false;
  core.transcript = undefined;
  vi.spyOn(fakeBrowser.runtime, 'getManifest').mockReturnValue({ version: '0.9.2' } as never);
  await fakeBrowser.storage.local.set({ 'vexdom/sessionKey': SESSION_KEY });
  worker = await import('./socket');
});

afterEach(() => worker?.disconnectDaemon());

async function until<T>(read: () => T | undefined): Promise<T> {
  for (let tries = 0; tries < 200; tries++) {
    const value = read();
    if (value !== undefined) return value;
    await new Promise((resolve) => setTimeout(resolve, 10));
  }
  throw new Error('timed out');
}

describe('the link through ext-core', () => {
  test('is welcomed, then answers a page call with the page result', async () => {
    await worker!.connectDaemon();
    await until(() => (worker!.daemonSpeaks(27) ? true : undefined));
    arrive({ tag: 'PageCall', callId: 7, kind: 'invoke', action: 'page.clickElement', input: '{"target":"#go"}', tabId: 3, runId: '' });
    const [callId, result] = await until(() => core.replies[0]);
    expect(callId).toBe(7);
    expect(result).toEqual({ ok: true, data: { action: 'page.clickElement', input: { target: '#go' }, tabId: 3 } });
  });

  test('sends a screenshot as bytes, its result without the data URL', async () => {
    await worker!.connectDaemon();
    await until(() => (worker!.daemonSpeaks(27) ? true : undefined));
    arrive({ tag: 'PageCall', callId: 9, kind: 'screenshot', action: 'page.screenshot', input: '{}', tabId: -1, runId: '' });
    const [callId, result, image] = await until(() => core.shots[0]);
    expect(callId).toBe(9);
    expect(result).toEqual({ ok: true, data: { format: 'png', width: 1, height: 1 } });
    expect([...image]).toEqual([1, 2, 3]);
  });

  test('refuses a page call on a link the Bridge has not proved itself on', async () => {
    core.withholdWelcome = true;
    void worker!.connectDaemon();
    await until(() => core.transcript);
    await new Promise((resolve) => setTimeout(resolve, 20));
    arrive({ tag: 'PageCall', callId: 11, kind: 'invoke', action: 'page.getPageInfo', input: '{}', tabId: -1, runId: '' });
    const [callId, result] = await until(() => core.replies.find(([id]) => id === 11));
    expect(callId).toBe(11);
    expect(result).toMatchObject({ ok: false, error: { code: 'UNAUTHORIZED' } });
    expect(worker!.daemonSpeaks(27)).toBe(false);
  });
});
