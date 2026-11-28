import { z } from 'zod';
import { ActionError } from '../core';
import { resolveRef } from './snapshot';

/**
 * Marks a field the extension accepts but does not describe: the tool list leaves it out
 * (`describeActions`), since only the Vexdom Bridge fills it in.
 */
export const UNDESCRIBED = 'x-vexdom-undescribed';

/** An element of a `page.snapshot`, as the Bridge resolves the agent's `e5` (crates/vexdom-daemon/src/page.rs). */
const refSchema = z
  .object({
    doc: z.string(),
    id: z.number().int().nonnegative(),
    frameId: z.number().int().optional(),
    role: z.string().optional(),
    name: z.string().optional(),
    nth: z.number().int().nonnegative().optional(),
  })
  .meta({ [UNDESCRIBED]: true });

export type TargetRef = z.output<typeof refSchema>;

export const targetSchema = z.object({
  selector: z.string().optional().describe('CSS selector for the element'),
  text: z
    .string()
    .optional()
    .describe(
      'Case-insensitive visible text the element should contain; when it is the whole text of some, only those match',
    ),
  role: z.string().optional().describe('Tag name or ARIA role to narrow matches, e.g. "button" or "link"'),
  label: z
    .string()
    .optional()
    .describe('Case-insensitive text of the element’s <label>, aria-label or aria-labelledby'),
  placeholder: z.string().optional().describe('Case-insensitive text the placeholder contains'),
  alt: z.string().optional().describe('Case-insensitive text the alt attribute contains'),
  title: z.string().optional().describe('Case-insensitive text the title attribute contains'),
  testId: z.string().optional().describe('Exact data-testid attribute'),
  name: z.string().optional().describe('Case-insensitive accessible name the element has exactly; use with "role"'),
  nth: z.number().int().nonnegative().default(0).describe('Zero-based index when several elements match'),
  last: z.boolean().optional().describe('Pick the last match instead of the nth'),
  exact: z
    .boolean()
    .optional()
    .describe('text, label, placeholder, alt, title and name match the whole text, case-sensitively'),
  ref: refSchema.optional(),
});

export type Target = z.output<typeof targetSchema>;

const INTERACTIVE =
  'a,button,input,select,textarea,summary,label,h1,h2,h3,h4,h5,h6,[role],[tabindex],[contenteditable]';

const ROLE_TAGS: Record<string, string[]> = {
  link: ['a'],
  button: ['button'],
  textbox: ['input', 'textarea'],
  combobox: ['select'],
  checkbox: ['input'],
  radio: ['input'],
  heading: ['h1', 'h2', 'h3', 'h4', 'h5', 'h6'],
};

const ROLE_BY_INPUT_TYPE: Record<string, string> = {
  button: 'button',
  submit: 'button',
  reset: 'button',
  image: 'button',
  file: 'button',
  checkbox: 'checkbox',
  radio: 'radio',
  range: 'slider',
  number: 'spinbutton',
  search: 'searchbox',
};

const ROLE_BY_TAG: Record<string, string> = {
  button: 'button',
  select: 'combobox',
  textarea: 'textbox',
  summary: 'button',
  option: 'option',
  h1: 'heading',
  h2: 'heading',
  h3: 'heading',
  h4: 'heading',
  h5: 'heading',
  h6: 'heading',
  nav: 'navigation',
  main: 'main',
  header: 'banner',
  footer: 'contentinfo',
  aside: 'complementary',
  form: 'form',
  dialog: 'dialog',
};

const VALUELESS_INPUT = new Set(['button', 'submit', 'reset', 'image', 'checkbox', 'radio', 'file']);

export function computedRole(el: Element): string | undefined {
  const explicit = el.getAttribute('role')?.trim().split(/\s+/)[0];
  if (explicit) return explicit.toLowerCase();
  if (el instanceof HTMLInputElement) return ROLE_BY_INPUT_TYPE[el.type] ?? 'textbox';
  if (el instanceof HTMLAnchorElement) return el.getAttribute('href') === null ? undefined : 'link';
  return ROLE_BY_TAG[el.tagName.toLowerCase()];
}

function matchesRole(el: Element, role: string): boolean {
  const wanted = role.toLowerCase();
  const tag = el.tagName.toLowerCase();
  if (tag === wanted) return true;
  if (computedRole(el) === wanted) return true;
  return ROLE_TAGS[wanted]?.includes(tag) ?? false;
}

/** Elements a locator can name besides the interactive ones: images, titled and test-id'd elements. */
const LOCATABLE = `${INTERACTIVE},img,[alt],[title],[data-testid]`;

/** Whether a target names anything at all; a ref or one of the locator fields. */
export function hasLocator(target: Partial<Target>): boolean {
  const { selector, text, role, label, placeholder, alt, title, testId, ref } = target;
  return !!(ref || selector || text || role || label || placeholder || alt || title || testId);
}

const collapsed = (text: string) => text.replace(/\s+/g, ' ').trim().toLowerCase();

const squashed = (text: string) => text.replace(/\s+/g, ' ').trim();

/** Case-insensitive containment, or with `exact` the whole text, whitespace collapsed, case kept. */
const contains = (value: string | null | undefined, needle: string, exact = false) =>
  !!value && (exact ? squashed(value) === squashed(needle) : value.toLowerCase().includes(needle.toLowerCase()));

/** The `<label>`, `aria-label` and `aria-labelledby` texts of an element, joined. */
export function labelText(el: Element): string {
  const parts: string[] = [];
  const labels = (el as HTMLInputElement).labels;
  if (labels) parts.push(...[...labels].map((label) => label.textContent ?? ''));
  const aria = el.getAttribute('aria-label');
  if (aria) parts.push(aria);
  for (const id of (el.getAttribute('aria-labelledby') ?? '').split(/\s+/).filter(Boolean)) {
    parts.push(el.ownerDocument.getElementById(id)?.textContent ?? '');
  }
  return parts.join(' ').replace(/\s+/g, ' ').trim();
}

/** Every element that passes every field the target gives, in document order. */
export function resolveAll(target: Target, options: { includeHidden?: boolean } = {}): HTMLElement[] {
  // A ref can name any element, an SVG icon say: callers that need more than Element check for it.
  if (target.ref) return [resolveRef(target.ref) as HTMLElement];
  if (!hasLocator(target)) {
    throw new ActionError(
      'A target needs a "selector", "text", "role", "label", "placeholder", "alt", "title" or "testId"',
      'INVALID_TARGET',
    );
  }
  const { selector, text, role, label, placeholder, alt, title, testId, name, exact } = target;
  let pool = [...document.querySelectorAll<HTMLElement>(selector ?? LOCATABLE)];
  if (testId !== undefined) pool = pool.filter((el) => el.getAttribute('data-testid') === testId);
  if (role) pool = pool.filter((el) => matchesRole(el, role));
  if (label !== undefined) pool = pool.filter((el) => contains(labelText(el), label, exact));
  if (placeholder !== undefined)
    pool = pool.filter((el) => contains(el.getAttribute('placeholder'), placeholder, exact));
  if (alt !== undefined) pool = pool.filter((el) => contains(el.getAttribute('alt'), alt, exact));
  if (title !== undefined) pool = pool.filter((el) => contains(el.getAttribute('title'), title, exact));
  if (name !== undefined) {
    pool = pool.filter((el) =>
      exact ? squashed(accessibleText(el)) === squashed(name) : collapsed(accessibleText(el)) === collapsed(name),
    );
  }
  if (text) {
    pool = pool.filter((el) => contains(accessibleText(el), text, exact));
    pool = pool.filter((el) => !pool.some((other) => other !== el && el.contains(other)));
  }
  if (!options.includeHidden) pool = pool.filter(isVisible);
  if (text && !exact) {
    // "Technicals" means the button that says so, not "More technicals" beside it.
    const whole = pool.filter((el) => collapsed(accessibleText(el)) === collapsed(text));
    if (whole.length) pool = whole;
  }
  return pool;
}

export function resolveTarget(target: Target, options: { includeHidden?: boolean } = {}): HTMLElement {
  const pool = resolveAll(target, options);
  const element = target.last ? pool.at(-1) : pool[target.nth];
  if (!element) {
    throw new ActionError(
      `No element matches ${JSON.stringify(target)} (${pool.length} candidates)`,
      'TARGET_NOT_FOUND',
    );
  }
  return element;
}

/** `.checked` for a native checkbox or radio, else `aria-checked`. */
export function isChecked(el: Element): boolean {
  if (el instanceof HTMLInputElement && (el.type === 'checkbox' || el.type === 'radio')) return el.checked;
  return el.getAttribute('aria-checked') === 'true';
}

export function accessibleText(el: Element): string {
  const aria = el.getAttribute('aria-label')?.trim();
  if (aria) return aria;
  if (el instanceof HTMLInputElement) {
    if (el.type === 'submit' || el.type === 'button' || el.type === 'reset') {
      return el.value.trim() || el.type;
    }
    return fieldName(el);
  }
  if (el instanceof HTMLTextAreaElement || el instanceof HTMLSelectElement) return fieldName(el);
  if (el instanceof HTMLImageElement) return el.alt.trim();
  const text = (el.textContent ?? '').replace(/\s+/g, ' ').trim();
  if (text) return text;
  const img = el.querySelector('img[alt]');
  return img instanceof HTMLImageElement ? img.alt.trim() : '';
}

function fieldName(el: HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement): string {
  const label = el.labels?.[0]?.textContent?.trim();
  return label || el.getAttribute('placeholder')?.trim() || el.name;
}

export function isVisible(el: Element): boolean {
  const rect = el.getBoundingClientRect();
  if (rect.width === 0 && rect.height === 0) return false;
  const style = getComputedStyle(el);
  return style.visibility !== 'hidden' && style.display !== 'none';
}

export function isExposed(el: Element): boolean {
  return isVisible(el) && !el.closest('[aria-hidden="true"],[inert]');
}

function ariaFlag(el: Element, name: string): boolean | undefined {
  const value = el.getAttribute(name);
  return value === 'true' ? true : value === 'false' ? false : undefined;
}

export interface ElementState {
  disabled?: boolean;
  checked?: boolean;
  expanded?: boolean;
  selected?: boolean;
  current?: string;
  required?: boolean;
  invalid?: boolean;
  filled?: boolean;
  value?: string;
}

export function elementState(el: Element): ElementState | undefined {
  const state: ElementState = {};
  if (el.matches(':disabled') || ariaFlag(el, 'aria-disabled')) state.disabled = true;

  const checked =
    el instanceof HTMLInputElement && (el.type === 'checkbox' || el.type === 'radio')
      ? el.checked
      : ariaFlag(el, 'aria-checked');
  if (checked !== undefined) state.checked = checked;

  const expanded = ariaFlag(el, 'aria-expanded');
  if (expanded !== undefined) state.expanded = expanded;

  const selected = el instanceof HTMLOptionElement ? el.selected : ariaFlag(el, 'aria-selected');
  if (selected !== undefined) state.selected = selected;

  const current = el.getAttribute('aria-current');
  if (current && current !== 'false') state.current = current;

  if (el.matches(':required') || ariaFlag(el, 'aria-required')) state.required = true;
  if (ariaFlag(el, 'aria-invalid')) state.invalid = true;

  if (el instanceof HTMLSelectElement) {
    state.value = el.selectedOptions[0]?.text.replace(/\s+/g, ' ').trim().slice(0, 60) || undefined;
  } else if (el instanceof HTMLTextAreaElement) {
    state.filled = el.value.length > 0;
  } else if (el instanceof HTMLInputElement && !VALUELESS_INPUT.has(el.type)) {
    state.filled = el.value.length > 0;
  }

  return Object.values(state).some((value) => value !== undefined) ? state : undefined;
}

export function cssPath(el: Element): string {
  const path: string[] = [];
  for (let node: Element | null = el; node && node !== document.documentElement; node = node.parentElement) {
    if (node.id) {
      path.unshift(`#${CSS.escape(node.id)}`);
      return path.join(' > ');
    }
    const tag = node.tagName.toLowerCase();
    const siblings = node.parentElement
      ? [...node.parentElement.children].filter((child) => child.tagName === node!.tagName)
      : [];
    path.unshift(siblings.length > 1 ? `${tag}:nth-of-type(${siblings.indexOf(node) + 1})` : tag);
  }
  return path.join(' > ');
}

export function documentBounds(el: Element) {
  const rect = el.getBoundingClientRect();
  return {
    x: Math.round(rect.x + window.scrollX),
    y: Math.round(rect.y + window.scrollY),
    width: Math.round(rect.width),
    height: Math.round(rect.height),
  };
}

export function describeElement(el: Element) {
  const text = accessibleText(el).slice(0, 80);
  return {
    tag: el.tagName.toLowerCase(),
    role: computedRole(el),
    selector: cssPath(el),
    text: text || undefined,
    state: elementState(el),
    bounds: documentBounds(el),
  };
}

export function submitsOnClick(el: Element): boolean {
  if (!el.closest('form')) return false;
  if (el instanceof HTMLButtonElement) return el.type === 'submit';
  return el instanceof HTMLInputElement && (el.type === 'submit' || el.type === 'image');
}
