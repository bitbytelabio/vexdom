import { useCallback, useEffect, useState } from 'react';
import { browser } from 'wxt/browser';

import { Row, SettingsGroup } from '@/extension/components/extension-settings';
import { Button } from '@/extension/components/ui/button';
import { BRIDGE_CHANNEL, type ActionResult } from '@/lib/actions/protocol';
import { formatWhen } from '@/lib/format-when';
import type { ApprovalGrant } from '@/lib/settings/guardrails';

interface SiteGrants {
  host: string;
  actions: string[];
  newest: number;
}

const actionLabel = (action: string) => (action === '*' ? 'everything' : action.replace(/^page\./, ''));

/** One row per site, its actions in the order granted, newest site first. */
function bySite(grants: readonly ApprovalGrant[]): SiteGrants[] {
  const sites = new Map<string, SiteGrants>();
  for (const grant of grants) {
    const at = Date.parse(grant.at) || 0;
    const site = sites.get(grant.host) ?? { host: grant.host, actions: [], newest: 0 };
    site.actions.push(actionLabel(grant.action));
    site.newest = Math.max(site.newest, at);
    sites.set(grant.host, site);
  }
  return [...sites.values()].sort((a, b) => b.newest - a.newest);
}

const ask = <T,>(message: Record<string, unknown>) =>
  browser.runtime.sendMessage({ channel: BRIDGE_CHANNEL, ...message }) as Promise<ActionResult<T> | undefined>;

/** The sites where the user chose "Always on …" on an approval card, each revocable. */
export function ApprovalGrants() {
  const [sites, setSites] = useState<SiteGrants[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    const result = await ask<{ grants: ApprovalGrant[] }>({ op: 'approvals' });
    if (result?.ok) {
      setSites(bySite(result.data.grants));
      setError(null);
    } else setError(result?.error.message ?? 'Could not read the approvals.');
  }, []);

  useEffect(() => void load(), [load]);

  async function revoke(host?: string) {
    const result = await ask<{ removed: number }>({ op: 'forgetApprovals', ...(host ? { host } : {}) });
    if (result && !result.ok) setError(result.error.message);
    await load();
  }

  return (
    <SettingsGroup
      title="Always allowed"
      note="Sites where you chose “Always on …” on an approval card. Revoking makes Vexdom ask again."
    >
      {error ? (
        <Row title={error} />
      ) : !sites ? (
        <Row title="Reading…" />
      ) : sites.length === 0 ? (
        <Row title="Nothing yet." />
      ) : (
        <>
          {sites.map((site) => (
            <Row
              key={site.host}
              title={site.host}
              note={`${site.actions.join(', ')}${site.newest ? ` · ${formatWhen(site.newest)}` : ''}`}
            >
              <Button variant="ghost" size="sm" onClick={() => void revoke(site.host)}>
                Revoke
              </Button>
            </Row>
          ))}
          <div className="flex justify-end px-4 py-2.5">
            <Button variant="ghost" size="sm" onClick={() => void revoke()}>
              Revoke all
            </Button>
          </div>
        </>
      )}
    </SettingsGroup>
  );
}
