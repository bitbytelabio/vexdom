import { z } from 'zod';
import { ActionError, defineAction } from '../core';
import { describeElement, resolveTarget, targetSchema } from './dom';

const isEditable = (el: Element | null): el is HTMLElement =>
  el instanceof HTMLInputElement ||
  el instanceof HTMLTextAreaElement ||
  (el instanceof HTMLElement && el.isContentEditable);

/** Inserts at the caret as a paste or an IME would: one `input` event, no key events. */
export function insertAtCaret(el: HTMLElement, text: string): void {
  if (document.execCommand('insertText', false, text)) return;
  if (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement) {
    const start = el.selectionStart ?? el.value.length;
    const end = el.selectionEnd ?? el.value.length;
    el.setRangeText(text, start, end, 'end');
    el.dispatchEvent(new InputEvent('input', { bubbles: true, data: text, inputType: 'insertText' }));
    return;
  }
  throw new ActionError('The focused element did not accept the text.', 'NOT_EDITABLE');
}

/** The element to type into: the target, focused, or whatever has the focus now. */
export function editableTarget(target: z.output<typeof targetSchema> | undefined): HTMLElement {
  const el = target ? resolveTarget(target) : document.activeElement;
  if (target && el instanceof HTMLElement) el.focus();
  if (!isEditable(el)) throw new ActionError('No editable element has the focus — give a "target".', 'NOT_EDITABLE');
  return el;
}

export const insertText = defineAction({
  name: 'page.insertText',
  group: 'input',
  description:
    'Insert text at the caret without key events, as a paste or an input method would: for editors that mishandle synthetic key presses, or to add to a field without clearing it.',
  input: z.object({
    text: z.string().min(1).max(20_000).describe('The text to insert'),
    target: targetSchema.optional().describe('Field to focus first; defaults to the focused element'),
  }),
  execute({ text, target }) {
    const el = editableTarget(target);
    insertAtCaret(el, text);
    return { inserted: text.length, element: describeElement(el) };
  },
});
