import { z } from 'zod';
import { ActionError, defineAction } from '../core';
import { isDisabled } from './click-element';
import { cssPath, describeElement, resolveTarget, targetSchema } from './dom';
import { keyboardInit } from './keyboard';

export const fillInput = defineAction({
  name: 'page.fillInput',
  description: 'Fill a text input, textarea, or contenteditable element like a user typing.',
  input: z.object({
    target: targetSchema.describe('Element to type into'),
    value: z.string().describe('Text to enter'),
    clear: z.boolean().default(true).describe('Replace existing content instead of appending'),
    pressEnter: z.boolean().default(false).describe('Press Enter afterwards, which submits many forms'),
  }),
  execute({ target, value, clear, pressEnter }) {
    const el = resolveTarget(target);
    el.focus();
    const { length } = typeInto(el, value, { clear, pressEnter });
    return { element: describeElement(el), length, appended: !clear };
  },
});

/** Input types whose value has one fixed shape: [pattern, how to say it, an example]. */
const FORMATS: Record<string, [RegExp, string, string]> = {
  date: [/^\d{4}-\d{2}-\d{2}$/, 'YYYY-MM-DD', '2026-11-26'],
  time: [/^\d{2}:\d{2}(:\d{2}(\.\d{1,3})?)?$/, 'HH:MM or HH:MM:SS', '13:45'],
  'datetime-local': [/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}(:\d{2})?$/, 'YYYY-MM-DDTHH:MM', '2026-11-26T13:45'],
  month: [/^\d{4}-\d{2}$/, 'YYYY-MM', '2026-11'],
  week: [/^\d{4}-W\d{2}$/, 'YYYY-Www', '2026-W48'],
  color: [/^#[0-9a-fA-F]{6}$/, '#rrggbb', '#ff8800'],
};

const invalid = (type: string, format: string, example: string) =>
  new ActionError(`${type} inputs take ${format}, e.g. ${example}`, 'INVALID_INPUT');

/** A value an `<input>` of a fixed-format type accepts, or why not. */
function checkFormatted(el: HTMLInputElement, value: string): void {
  const { type } = el;
  const format = FORMATS[type];
  if (format && !format[0].test(value)) throw invalid(type, format[1], format[2]);
  if (type === 'number' || type === 'range') {
    const number = Number(value);
    if (value.trim() === '' || !Number.isFinite(number)) throw invalid(type, 'a number', '42');
    if (type === 'range') {
      const min = el.min === '' ? 0 : Number(el.min);
      const max = el.max === '' ? 100 : Number(el.max);
      if (number < min || number > max) throw invalid(type, `a number from ${min} to ${max}`, String(min));
    }
  }
}

const FORMATTED = new Set([...Object.keys(FORMATS), 'number', 'range']);

function refuseUneditable(el: HTMLElement): void {
  const path = cssPath(el);
  if ((el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement) && el.readOnly) {
    throw new ActionError(`${path} is read-only`, 'INVALID_TARGET');
  }
  if (isDisabled(el)) throw new ActionError(`${path} is disabled`, 'INVALID_TARGET');
  if (el instanceof HTMLInputElement && (el.type === 'checkbox' || el.type === 'radio')) {
    throw new ActionError(`${path} is a ${el.type}; use check or uncheck`, 'INVALID_TARGET');
  }
  if (el instanceof HTMLSelectElement) throw new ActionError(`${path} is a select; use selectOption`, 'INVALID_TARGET');
}

export function typeInto(
  el: HTMLElement,
  value: string,
  { clear, pressEnter }: { clear: boolean; pressEnter: boolean },
): { length: number; submitted: boolean } {
  refuseUneditable(el);
  let next: string;
  if (el instanceof HTMLInputElement && FORMATTED.has(el.type)) {
    // A date, a colour or a number is set whole: appending to one makes no sense.
    checkFormatted(el, value);
    next = value;
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(el, next);
    if (el.value !== value) {
      throw new ActionError(`the browser refused "${value}" for this ${el.type} input`, 'INVALID_INPUT');
    }
    el.dispatchEvent(new Event('input', { bubbles: true }));
    el.dispatchEvent(new Event('change', { bubbles: true }));
  } else if (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement) {
    next = clear ? value : el.value + value;
    const proto = el instanceof HTMLInputElement ? HTMLInputElement.prototype : HTMLTextAreaElement.prototype;
    Object.getOwnPropertyDescriptor(proto, 'value')!.set!.call(el, next);
    el.dispatchEvent(new InputEvent('input', { bubbles: true, data: value, inputType: 'insertText' }));
    el.dispatchEvent(new Event('change', { bubbles: true }));
  } else if (el.isContentEditable) {
    if (!editByCommand(el, value, clear)) {
      el.textContent = clear ? value : (el.textContent ?? '') + value;
      el.dispatchEvent(new InputEvent('input', { bubbles: true, data: value, inputType: 'insertText' }));
    }
    next = el.textContent ?? '';
  } else {
    throw new ActionError(`<${el.tagName.toLowerCase()}> is not an editable element`, 'INVALID_TARGET');
  }
  return { length: next.length, submitted: pressEnter ? pressEnterIn(el) : false };
}

/**
 * Puts `value` into a contenteditable through the browser's own editing command, the path a
 * user's typing takes: rich editors (ProseMirror, Lexical, Slate) keep their model in step only
 * with it. Replaces everything with `replaceAll`, else inserts at the end; an empty value deletes.
 * False when the browser would not run the command, so the caller sets the text itself.
 */
export function editByCommand(el: HTMLElement, value: string, replaceAll: boolean): boolean {
  if (typeof document.execCommand !== 'function') return false;
  const selection = window.getSelection();
  if (!selection) return false;
  el.focus({ preventScroll: true });
  const range = document.createRange();
  range.selectNodeContents(el);
  if (!replaceAll) range.collapse(false);
  selection.removeAllRanges();
  selection.addRange(range);
  if (value === '' && !replaceAll) return true;
  return document.execCommand(value === '' ? 'delete' : 'insertText', false, value);
}

function pressEnterIn(el: HTMLElement): boolean {
  const enter = keyboardInit('Enter');
  const notPrevented = el.dispatchEvent(new KeyboardEvent('keydown', enter));
  el.dispatchEvent(new KeyboardEvent('keyup', enter));
  const form = notPrevented && !(el instanceof HTMLTextAreaElement) ? el.closest('form') : null;
  form?.requestSubmit();
  return !!form;
}
