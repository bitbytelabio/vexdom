import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'vitest';
import { TOOLS_TARGETS, toolsFileText } from './emit';
import { actions, describeActions } from './registry';

const DEBUGGER_ONLY = [
  'page.trustedClick',
  'page.findCaptcha',
  'page.solveCaptcha',
  'page.startDiagnostics',
  'page.readConsole',
  'page.readNetwork',
  'page.stopDiagnostics',
  'page.injectCode',
  'page.runCode',
  'page.mouse',
  'page.initScript',
  'page.routeNetwork',
  'page.unrouteNetwork',
  'page.harStart',
  'page.harStop',
  'page.dialog',
  'page.emulate',
  'page.tabKeep',
  'page.pdf',
  'page.recordStart',
  'page.recordStop',
  'page.traceStart',
  'page.traceStop',
  'page.vitals',
  'page.react',
  'page.a11yAudit',
];

const names = (tools: { name: string }[]) => tools.map(({ name }) => name);

describe('the list a build offers', () => {
  test('Chromium is offered every action', () => {
    expect(names(describeActions('chromium'))).toEqual([...actions.keys()]);
    expect(names(describeActions())).toEqual(names(describeActions('chromium')));
  });

  test("Firefox is offered everything but the tools that need Chrome's debugger", () => {
    const offered = names(describeActions('firefox'));
    const hidden = [...actions.keys()].filter((name) => !offered.includes(name));
    expect(hidden.sort()).toEqual([...DEBUGGER_ONLY].sort());
  });

  test('a hidden tool is still invocable, so a stale caller gets its own hint', () => {
    for (const name of DEBUGGER_ONLY) expect(actions.has(name)).toBe(true);
  });

  test('no description still tells Chrome about Firefox', () => {
    expect(JSON.stringify(describeActions('chromium'))).not.toContain('Firefox');
  });
});

const schemaOf = (name: string) =>
  describeActions().find((action) => action.name === name)!.inputSchema as {
    properties: Record<string, { properties?: Record<string, Record<string, unknown>> } & Record<string, unknown>>;
  };

describe('the described schemas', () => {
  test('none carries the JSON Schema dialect URL', () => {
    expect(JSON.stringify(describeActions())).not.toContain('$schema');
  });

  test('none carries the safe-integer bounds zod implies for an integer', () => {
    expect(JSON.stringify(describeActions())).not.toContain(String(Number.MAX_SAFE_INTEGER));
  });

  test("a target's ref is accepted but left out, as only the Vexdom Bridge fills it in", () => {
    expect(schemaOf('page.clickElement').properties.target.properties).not.toHaveProperty('ref');
    expect(JSON.stringify(describeActions())).not.toContain('x-vexdom-undescribed');
    const input = { target: { ref: { doc: 'd', id: 1, frameId: 0, role: 'button', name: 'Go' } } };
    expect(actions.get('page.clickElement')!.input.safeParse(input).success).toBe(true);
  });

  test('a bound the action declared itself is kept', () => {
    expect(schemaOf('page.clickElement').properties.target.properties!.nth).toMatchObject({ type: 'integer', minimum: 0 });
    expect(schemaOf('page.dragElement').properties.steps).toMatchObject({ minimum: 2, maximum: 60 });
  });
});

describe('the manifest the Rust Bridge embeds', () => {
  test.each(TOOLS_TARGETS)('crates/vexdom-core/assets/tools.%s.json matches the registry (run `bun run tools:emit`)', (target) => {
    const emitted = readFileSync(new URL(`../../../../../crates/vexdom-core/assets/tools.${target}.json`, import.meta.url), 'utf8');
    expect(emitted).toBe(toolsFileText(target));
  });
});
