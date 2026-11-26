import { z } from 'zod';
import { ActionError, defineAction } from '../core';
import { editableTarget, insertAtCaret } from './insert-text';

/** Runs `use` with a hidden textarea focused, then gives the focus and the selection back. */
function withScratch<T>(use: (area: HTMLTextAreaElement) => T): T {
  const focused = document.activeElement as HTMLElement | null;
  const selection = document.getSelection();
  const ranges = selection ? Array.from({ length: selection.rangeCount }, (_, at) => selection.getRangeAt(at)) : [];
  const area = document.createElement('textarea');
  area.setAttribute('aria-hidden', 'true');
  area.style.cssText = 'position:fixed;top:0;left:0;width:1px;height:1px;opacity:0;pointer-events:none';
  document.documentElement.append(area);
  try {
    area.focus();
    return use(area);
  } finally {
    area.remove();
    focused?.focus?.({ preventScroll: true });
    if (selection && ranges.length) {
      selection.removeAllRanges();
      for (const range of ranges) selection.addRange(range);
    }
  }
}

async function readClipboard(): Promise<string> {
  if (import.meta.env.FIREFOX) return navigator.clipboard.readText();
  return withScratch((area) => {
    document.execCommand('paste');
    return area.value;
  });
}

async function writeClipboard(text: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    const copied = withScratch((area) => {
      area.value = text;
      area.select();
      return document.execCommand('copy');
    });
    if (!copied) throw new ActionError('The browser refused to write the clipboard.', 'CLIPBOARD_FAILED');
  }
}

export const clipboard = defineAction({
  name: 'page.clipboard',
  group: 'storage',
  description:
    'Use the system clipboard from the page: read it, write text to it, copy the current selection, or paste it at the caret.',
  input: z.object({
    op: z.enum(['read', 'write', 'copy', 'paste']).describe('What to do'),
    text: z.string().optional().describe('The text for "write"'),
  }),
  async execute({ op, text }) {
    switch (op) {
      case 'write':
        if (text === undefined) throw new ActionError('"write" needs the "text"', 'INVALID_INPUT');
        await writeClipboard(text);
        return { written: text.length };
      case 'copy':
        return { copied: document.execCommand('copy'), text: document.getSelection()?.toString() ?? '' };
      case 'read':
        return { text: await readClipboard() };
      case 'paste': {
        const el = editableTarget(undefined);
        const pasted = await readClipboard();
        el.focus();
        insertAtCaret(el, pasted);
        return { pasted: pasted.length };
      }
    }
  },
});
