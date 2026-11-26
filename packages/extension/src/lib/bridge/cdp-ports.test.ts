import { describe, expect, it } from 'vitest';
import { harMimeIsText, harTimings } from './har';
import { namedKeyInfo, pressKeyWithChord, singleKeyEventParams } from './keys';
import { mockedResponse, routeUrlMatches, selectRoute } from './network-route';
import { planPointerMove } from './trusted-input';

describe('route matching (agent-browser route_url_matches)', () => {
  it.each([
    ['*', 'https://a.test/x', true],
    ['/api/', 'https://a.test/api/items', true],
    ['*api*', 'https://a.test/api/items', true],
    ['https://a.test/*', 'https://a.test/x', true],
    ['https://a.test/*', 'https://b.test/x', false],
    ['*.png', 'https://a.test/i.png', true],
    ['*.png', 'https://a.test/i.png?v=1', false],
    ['**api**items', 'https://a.test/api/items', true],
  ])('%s against %s is %s', (pattern, url, expected) => {
    expect(routeUrlMatches(pattern, url)).toBe(expected);
  });

  it('picks the first route that matches and acts', () => {
    const routes = [
      { pattern: '*api*', abort: false, resourceTypes: ['Image'] },
      { pattern: '*api*', abort: true, resourceTypes: [] },
    ];
    expect(selectRoute(routes, 'https://a.test/api', 'XHR')).toBe(routes[1]);
  });
});

describe('a mocked response', () => {
  it('labels JSON and gives CORS headers only to the allowed origin', () => {
    const [status, headers, body] = mockedResponse({ body: '{"x":1}', allowOrigin: 'https://app.test/' }, { Origin: 'https://app.test' });
    expect(status).toBe(200);
    expect(atob(body)).toBe('{"x":1}');
    expect(headers).toContainEqual({ name: 'Content-Type', value: 'application/json' });
    expect(headers).toContainEqual({ name: 'Access-Control-Allow-Origin', value: 'https://app.test' });
    const [, other] = mockedResponse({ body: 'x', allowOrigin: 'https://app.test' }, { Origin: 'https://evil.test' });
    expect(other.some((header) => header.name.startsWith('Access-Control'))).toBe(false);
  });

  it('answers an allowed preflight with 204 and no body', () => {
    const [status, , body] = mockedResponse(
      { body: 'x', allowOrigin: 'https://app.test' },
      { origin: 'https://app.test', 'access-control-request-method': 'POST' },
    );
    expect([status, body]).toEqual([204, '']);
  });
});

describe('HAR helpers', () => {
  it('knows text bodies from binary ones', () => {
    expect(harMimeIsText('application/json; charset=utf-8')).toBe(true);
    expect(harMimeIsText('application/vnd.api+json')).toBe(true);
    expect(harMimeIsText('image/png')).toBe(false);
  });

  it('turns CDP resource timing into HAR phases', () => {
    const [timings, total] = harTimings(
      { requestTime: 10, dnsStart: 1, dnsEnd: 3, connectStart: 3, connectEnd: 8, sslStart: -1, sslEnd: -1, sendStart: 8, sendEnd: 9, receiveHeadersStart: 20, receiveHeadersEnd: 21 },
      10.05,
    );
    expect(timings).toMatchObject({ dns: 2, connect: 5, send: 1, wait: 11, blocked: 1 });
    expect(timings.receive).toBeCloseTo(29);
    expect(total).toBeCloseTo(1 + 2 + 5 + 1 + 11 + 29);
  });
});

describe('keys (agent-browser interaction.rs)', () => {
  it('maps names and characters on a US layout', () => {
    expect(namedKeyInfo('Return')).toEqual(['Enter', 'Enter', 13]);
    expect(namedKeyInfo('KeyA')).toEqual(['a', 'KeyA', 65]);
    expect(namedKeyInfo('.')).toEqual(['.', 'Period', 190]);
  });

  it('shifts the key and drops the text under Control', () => {
    expect(singleKeyEventParams('1', 'keyDown', 8)).toMatchObject({ key: '!', code: 'Digit1', text: '!' });
    expect(singleKeyEventParams('a', 'keyDown', 2)).not.toHaveProperty('text');
  });

  it('presses chord modifiers around the key and releases them', async () => {
    const sent: string[] = [];
    await pressKeyWithChord(async (params) => sent.push(`${params.type} ${params.key} ${params.modifiers ?? 0}`), 'a', 2, 0);
    expect(sent).toEqual(['keyDown Control 2', 'keyDown a 2', 'keyUp a 2', 'keyUp Control 0']);
  });
});

describe('pointer paths', () => {
  it('ends exactly on the target and bends only when human', () => {
    const straight = planPointerMove({ x: 0, y: 0 }, { x: 120, y: 0 }, { human: false, seed: 0 });
    expect(straight).toHaveLength(10);
    expect(straight.every((point) => point.y === 0)).toBe(true);
    const human = planPointerMove({ x: 0, y: 0 }, { x: 120, y: 0 }, { human: true, seed: 7 });
    expect(human.at(-1)).toMatchObject({ x: 120, y: 0 });
    expect(human.some((point) => point.y !== 0)).toBe(true);
  });
});
