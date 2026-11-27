import { AGENTS, activeRunner, type AgentState } from '@/lib/agents/catalog';

const REPOSITORY = 'https://github.com/bitbytelabio/vexdom';

export const LINKS = {
  author: 'https://github.com/nguyenthdat',
  upstream: 'https://github.com/imshaikot/browsentic',
  repository: REPOSITORY,
  site: REPOSITORY,
  guide: 'https://github.com/bitbytelabio/vexdom/tree/main/docs/guide/',
  pairing: 'https://github.com/bitbytelabio/vexdom/blob/main/docs/guide/install.md#3-pair',
  installGuide: 'https://github.com/bitbytelabio/vexdom/blob/main/docs/guide/install.md',
  troubleshooting: 'https://github.com/bitbytelabio/vexdom/blob/main/docs/guide/troubleshooting.md',
  changelog: `${REPOSITORY}/releases`,
  feature: `${REPOSITORY}/issues/new?template=capability.yml`,
} as const;

export const AUTHOR = 'nguyenthdat';

export const STAR_ASK =
  'Vexdom is free and open source — no API key, no subscription, no account. A star on GitHub is how the next person finds it.';

export interface VersionRow {
  label: string;
  value: string;
}

export const describeVersions = (rows: readonly VersionRow[]): string =>
  rows.map(({ label, value }) => `${label} ${value}`).join(' · ');

const RELEASE = /\d+(?:\.\d+)+(?:[-+][\w.]+)?/;

export function describeAgent(state: AgentState | undefined): string | undefined {
  const runner = activeRunner(state);
  if (!runner) return undefined;
  const release = runner.version && (runner.version.match(RELEASE)?.[0] ?? runner.version);
  return [AGENTS[runner.kind]?.label ?? runner.kind, release].filter(Boolean).join(' ');
}

export function bugReportUrl({ environment, agent }: { environment: string; agent?: string }): string {
  const fields = new URLSearchParams({ template: 'bug_report.yml', environment, ...(agent ? { agent } : {}) });
  return `${REPOSITORY}/issues/new?${fields}`;
}
