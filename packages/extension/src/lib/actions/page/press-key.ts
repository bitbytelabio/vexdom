import { z } from 'zod';
import { defineAction } from '../core';
import { describeElement, resolveTarget, targetSchema } from './dom';
import { keyboardInit } from './keyboard';

export const pressKey = defineAction({
  name: 'page.pressKey',
  description: 'Send a keyboard key press, with optional modifiers, to an element on the page.',
  input: z.object({
    key: z.string().describe('DOM KeyboardEvent.key value, e.g. "Enter", "Escape", "ArrowDown", "a"'),
    modifiers: z
      .array(z.enum(['ctrl', 'shift', 'alt', 'meta']))
      .default([])
      .describe('Modifier keys held during the press'),
    target: targetSchema
      .optional()
      .describe('Element to receive the key; defaults to the currently focused element'),
    phase: z
      .enum(['press', 'down', 'up'])
      .default('press')
      .describe('"down" only presses the key (keydown, and keypress for a printable key) and "up" only releases it'),
    trusted: z
      .boolean()
      .default(false)
      .describe('Send real browser key events (isTrusted), which select text with Control+a and trigger shortcuts; Chromium only.'),
  }),
  execute({ key, modifiers, target, phase }) {
    const el = target
      ? resolveTarget(target)
      : ((document.activeElement as HTMLElement | null) ?? document.body);
    try {
      el.focus();
    } catch {}
    const init = keyboardInit(key, modifiers);
    const printable = key.length === 1 && !init.ctrlKey && !init.metaKey;
    if (phase === 'up') {
      el.dispatchEvent(new KeyboardEvent('keyup', init));
      return { key, modifiers, phase, receivedBy: describeElement(el) };
    }
    const notPrevented = el.dispatchEvent(new KeyboardEvent('keydown', init));
    if (notPrevented && (printable || key === 'Enter')) {
      el.dispatchEvent(new KeyboardEvent('keypress', init));
    }
    if (phase === 'down') return { key, modifiers, phase, receivedBy: describeElement(el) };
    if (printable && (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement)) {
      const start = el.selectionStart ?? el.value.length;
      const end = el.selectionEnd ?? el.value.length;
      const next = el.value.slice(0, start) + key + el.value.slice(end);
      const proto = el instanceof HTMLInputElement ? HTMLInputElement.prototype : HTMLTextAreaElement.prototype;
      Object.getOwnPropertyDescriptor(proto, 'value')?.set?.call(el, next);
      const caret = start + key.length;
      try {
        el.setSelectionRange(caret, caret);
      } catch {}
      el.dispatchEvent(new InputEvent('input', { bubbles: true, data: key, inputType: 'insertText' }));
    }
    el.dispatchEvent(new KeyboardEvent('keyup', init));
    if (key === 'Enter' && notPrevented && el instanceof HTMLInputElement) el.form?.requestSubmit();
    return { key, modifiers, receivedBy: describeElement(el) };
  },
});
