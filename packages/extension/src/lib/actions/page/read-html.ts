import { z } from 'zod';
import { OVERLAY_ATTRIBUTE } from '@/lib/overlay';
import { defineAction } from '../core';
import { resolveTarget, targetSchema } from './dom';

/** What the Bridge's markdown drops anyway, cut before the HTML leaves the page. */
const DROPPED = `script, style, noscript, template, svg, [${OVERLAY_ATTRIBUTE}]`;

const SHOW_COMMENT = 0x80;

export const readHtml = defineAction({
  name: 'page.readHtml',
  description: 'The HTML of the page or of one element, for the Vexdom Bridge to turn into markdown.',
  input: z.object({
    target: targetSchema.optional().describe('Element to read; the whole document when omitted'),
  }),
  execute({ target }) {
    const element = target ? resolveTarget(target, { includeHidden: true }) : document.documentElement;
    return { url: location.href, title: document.title, html: htmlOf(element) };
  },
});

/**
 * The element's HTML without scripts, styles, icons, comments or the extension's own overlays.
 * The copy is made in a document with no browsing context, so no custom element of the page is
 * constructed for it.
 */
function htmlOf(element: Element): string {
  const inert = document.implementation.createHTMLDocument('');
  const copy = inert.importNode(element, true);
  for (const dropped of copy.querySelectorAll(DROPPED)) dropped.remove();
  const comments: Comment[] = [];
  const walker = inert.createTreeWalker(copy, SHOW_COMMENT);
  for (let comment = walker.nextNode(); comment; comment = walker.nextNode()) comments.push(comment as Comment);
  for (const comment of comments) comment.remove();
  return copy.outerHTML;
}
