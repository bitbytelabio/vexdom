/**
 * `vexdom screenshot --annotate`: numbered boxes over the elements a snapshot gave refs, drawn just
 * before the capture and removed right after. Each frame draws the labels of its own document;
 * boxes sit in document coordinates, so a full-page capture that scrolls keeps them in place.
 */
import { elementForId } from '@/lib/actions/page/snapshot';
import { OVERLAY_ATTRIBUTE } from '@/lib/overlay';

export interface RefLabel {
  doc: string;
  id: number;
  /** The ref as agents write it, e.g. `e5`. */
  label: string;
}

const HOST_ID = 'vexdom-ref-labels';
const COLOR = '#e11d48';

/** Draws the labels this document owns; resolves once they are painted, with how many it drew. */
export async function drawRefLabels(labels: readonly RefLabel[]): Promise<number> {
  clearRefLabels();
  const host = document.createElement('div');
  host.id = HOST_ID;
  host.setAttribute(OVERLAY_ATTRIBUTE, '');
  host.style.cssText = 'position:absolute;left:0;top:0;width:0;height:0;pointer-events:none;z-index:2147483647;';
  let drawn = 0;
  for (const { doc, id, label } of labels) {
    const element = elementForId(doc, id);
    if (!element) continue;
    const rect = element.getBoundingClientRect();
    if (!rect.width && !rect.height) continue;
    const box = document.createElement('div');
    box.style.cssText =
      `position:absolute;left:${rect.left + scrollX}px;top:${rect.top + scrollY}px;` +
      `width:${rect.width}px;height:${rect.height}px;box-sizing:border-box;border:2px solid ${COLOR};`;
    const tag = document.createElement('span');
    tag.textContent = label;
    tag.style.cssText =
      `position:absolute;left:-2px;top:-16px;padding:0 3px;font:bold 11px/14px monospace;` +
      `color:#fff;background:${COLOR};white-space:nowrap;`;
    box.append(tag);
    host.append(box);
    drawn++;
  }
  document.documentElement.append(host);
  // Two frames: the boxes are laid out and painted before the background captures.
  await new Promise<void>((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));
  return drawn;
}

export function clearRefLabels(): void {
  document.getElementById(HOST_ID)?.remove();
}
