/**
 * Names an agent gives tabs (`vexdom tab new --label docs`), so it can come back to one by name
 * rather than by id or title. Kept in `storage.session` (they mean nothing after a restart) and
 * dropped when the tab closes. The rules are agent-browser's (`native/browser.rs` `is_valid_label`).
 */
import { browser } from 'wxt/browser';

const LABELS_KEY = 'vexdom/tabLabels';

type Labels = Record<string, string>;

export async function tabLabels(): Promise<Labels> {
  const held = await browser.storage.session.get(LABELS_KEY).catch(() => ({}) as Record<string, unknown>);
  return (held[LABELS_KEY] as Labels | undefined) ?? {};
}

async function writeLabels(labels: Labels): Promise<void> {
  await browser.storage.session.set({ [LABELS_KEY]: labels });
}

/** Why `label` cannot name a new tab, or null. */
export function labelRefusal(label: string, labels: Labels): string | null {
  if (!/^[A-Za-z][A-Za-z0-9_-]*$/.test(label)) {
    return `Invalid tab label \`${label}\`; labels must start with a letter and contain only letters, digits, \`-\`, and \`_\``;
  }
  if (/^t\d+$/.test(label)) return `Tab label \`${label}\` uses syntax reserved for tab IDs`;
  if (Object.values(labels).includes(label)) {
    return `Label \`${label}\` is already used by another tab; labels must be unique within a session`;
  }
  return null;
}

export async function setTabLabel(tabId: number, label: string): Promise<void> {
  await writeLabels({ ...(await tabLabels()), [String(tabId)]: label });
}

export function serveTabLabels(): void {
  browser.tabs.onRemoved.addListener((tabId) => {
    void tabLabels().then((labels) => {
      if (!(String(tabId) in labels)) return;
      const { [String(tabId)]: _gone, ...rest } = labels;
      return writeLabels(rest);
    });
  });
}
