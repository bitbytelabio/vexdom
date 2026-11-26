import { TOOL_GROUPS } from './groups';
import { hashManifest } from './manifest';
import { actions, describeActions, type BrowserTarget } from './registry';

/**
 * The tool manifest as data for the Rust Bridge (ADR-025), which embeds it rather than computing it:
 * `crates/vexdom-core/assets/tools.<target>.json`, written by `bun run tools:emit`. Besides what the
 * extension describes (and its hash), it carries what only the registry knows: each action's group,
 * the groups' guide index, and which actions run only in Chromium.
 */
function toolsFile(target: BrowserTarget) {
  const tools = describeActions(target);
  return {
    hash: hashManifest(tools),
    tools,
    groups: Object.fromEntries([...actions.values()].flatMap((action) => (action.group ? [[action.name, action.group]] : []))),
    toolGroups: TOOL_GROUPS,
    chromiumOnly: [...actions.values()].filter((action) => action.chromiumOnly).map((action) => action.name),
  };
}

export const TOOLS_TARGETS: readonly BrowserTarget[] = ['chromium', 'firefox'];

export const toolsFileText = (target: BrowserTarget): string => `${JSON.stringify(toolsFile(target), null, 2)}\n`;
