import { z } from 'zod';
import { ActionError, defineAction } from '../core';
import { isChecked, isVisible, resolveAll, resolveTarget, targetSchema } from './dom';

const MAX_TEXT = 20_000;

const STYLES = [
  'display',
  'visibility',
  'opacity',
  'color',
  'background-color',
  'font-size',
  'font-weight',
  'position',
  'z-index',
  'width',
  'height',
] as const;

const PAGE_WIDE = new Set(['title', 'url', 'count']);

/** A field's value; never a password's, which the snapshot does not show either. */
function valueOf(el: HTMLElement): string | string[] | null {
  if (el instanceof HTMLInputElement && el.type === 'password') return null;
  if (el instanceof HTMLSelectElement) {
    return el.multiple ? [...el.selectedOptions].map((option) => option.value) : el.value;
  }
  if (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement) return el.value;
  return null;
}

export const query = defineAction({
  name: 'page.query',
  description:
    'Read one fact about an element or the page, cheaper than a snapshot: its text, form value (never a password’s), an attribute, how many elements match, its box, computed styles, whether it is visible, enabled or checked; or the page title or url.',
  input: z.object({
    target: targetSchema.optional().describe('The element; not needed for "title" and "url"'),
    what: z
      .enum(['text', 'value', 'attr', 'count', 'box', 'styles', 'visible', 'enabled', 'checked', 'title', 'url'])
      .describe('What to read'),
    name: z.string().optional().describe('The attribute for "attr" (required), or one CSS property for "styles"'),
  }),
  execute({ target, what, name }) {
    if (what === 'title') return { value: document.title };
    if (what === 'url') return { value: location.href };
    if (!target) throw new ActionError(`"${what}" needs a "target"`, 'INVALID_INPUT');
    if (what === 'count') return { value: resolveAll(target, { includeHidden: true }).length };
    if (what === 'attr' && !name) throw new ActionError('"attr" needs the attribute "name"', 'INVALID_INPUT');
    const el = resolveTarget(target, { includeHidden: !PAGE_WIDE.has(what) });
    switch (what) {
      case 'text':
        return { value: (el.innerText ?? el.textContent ?? '').trim().slice(0, MAX_TEXT) };
      case 'value':
        return { value: valueOf(el) };
      case 'attr':
        return { value: el instanceof HTMLInputElement && el.type === 'password' && name!.toLowerCase() === 'value' ? null : el.getAttribute(name!) };
      case 'box': {
        const rect = el.getBoundingClientRect();
        return { value: { x: rect.x, y: rect.y, width: rect.width, height: rect.height } };
      }
      case 'styles': {
        const style = getComputedStyle(el);
        if (name) return { value: style.getPropertyValue(name) };
        return { value: Object.fromEntries(STYLES.map((property) => [property, style.getPropertyValue(property)])) };
      }
      case 'visible':
        return { value: isVisible(el) };
      case 'enabled':
        return { value: !(el as HTMLInputElement).disabled && el.getAttribute('aria-disabled') !== 'true' };
      case 'checked':
        return { value: isChecked(el) };
    }
  },
});
