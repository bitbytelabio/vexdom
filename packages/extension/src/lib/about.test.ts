import { readFileSync } from 'node:fs';
import { describe, expect, test } from 'vitest';
import { LINKS, bugReportUrl, describeAgent, describeVersions } from './about';

const siteSlug = (heading: string): string =>
  heading
    .trim()
    .toLowerCase()
    .replace(/[^\w\s-]/g, '')
    .replace(/\s+/g, '-')
    .replace(/-+/g, '-')
    .replace(/^-|-$/g, '');

describe('the pairing link', () => {
  test('lands on a heading the install guide still has, as vexdom.com anchors it', () => {
    const url = new URL(LINKS.pairing);
    expect(url.origin + url.pathname).toBe('https://github.com/bitbytelabio/vexdom/blob/main/docs/guide/install.md');
    const headings = readFileSync(new URL('../../../../docs/guide/install.md', import.meta.url), 'utf8')
      .split('\n')
      .filter((line) => line.startsWith('## '))
      .map((line) => siteSlug(line.slice(3)));
    expect(headings).toContain(url.hash.slice(1));
  });
});

describe('the bug report the About page opens', () => {
  test('lands on the bug form with the environment and the agent already filled in', () => {
    const url = new URL(bugReportUrl({ environment: 'Extension 0.7.15 · Daemon 0.7.15 & up', agent: 'Codex 0.42.0' }));
    expect(url.origin + url.pathname).toBe('https://github.com/bitbytelabio/vexdom/issues/new');
    expect(Object.fromEntries(url.searchParams)).toEqual({
      template: 'bug_report.yml',
      environment: 'Extension 0.7.15 · Daemon 0.7.15 & up',
      agent: 'Codex 0.42.0',
    });
  });

  test('leaves the agent field to the reporter when no agent is known', () => {
    expect(new URL(bugReportUrl({ environment: 'Extension 0.7.15' })).searchParams.has('agent')).toBe(false);
  });

  test('reads the versions as one line, in the order they were listed', () => {
    expect(
      describeVersions([
        { label: 'App', value: '0.7.15' },
        { label: 'Protocol', value: '22' },
      ]),
    ).toBe('App 0.7.15 · Protocol 22');
  });

  test('names the active agent with the release its CLI printed, and nothing when none is active', () => {
    expect(
      describeAgent({
        active: 'claude',
        runners: [{ kind: 'claude', bin: 'claude', ready: true, version: '2.1.284 (Claude Code)' }],
      }),
    ).toBe('Claude Code 2.1.284');
    expect(
      describeAgent({
        active: 'claude',
        runners: [{ kind: 'claude', bin: 'claude', ready: true, version: 'claude-cli 0.155.1-beta.2' }],
      }),
    ).toBe('Claude Code 0.155.1-beta.2');
    expect(
      describeAgent({
        active: 'claude',
        runners: [{ kind: 'claude', bin: 'claude', ready: true, version: 'nightly' }],
      }),
    ).toBe('Claude Code nightly');
    expect(describeAgent({ active: 'claude', runners: [{ kind: 'claude', bin: 'claude', ready: false }] })).toBe(
      'Claude Code',
    );
    expect(describeAgent(undefined)).toBeUndefined();
  });
});
