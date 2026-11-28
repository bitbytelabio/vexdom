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
    // A target the agent named is read even when the page hides it.
    const filterHidden = !target || !hiddenKind(element);
    return { url: location.href, title: document.title, html: htmlOf(element, filterHidden) };
  },
});

/**
 * The element's HTML without what the page does not show (best effort: `display: none`,
 * `visibility: hidden`, `content-visibility: hidden`), scripts, styles, icons, comments or the
 * extension's own overlays; each link's `href` is the address the browser resolved. Hidden text is
 * left out both because a reader would not see it and because it is where injected instructions
 * hide. The copy is made in a document with no browsing context, so no custom element of the page
 * is constructed for it.
 */
function htmlOf(element: Element, filterHidden: boolean): string {
  const inert = document.implementation.createHTMLDocument('');
  const copy = inert.importNode(element, true);
  // The copy has the live tree's elements in the same order, so an index names both.
  const live = [element, ...element.querySelectorAll('*')];
  const copied = [copy as Element, ...copy.querySelectorAll('*')];
  if (live.length === copied.length) {
    live.forEach((el, index) => {
      if (el instanceof HTMLAnchorElement && el.hasAttribute('href')) copied[index].setAttribute('href', el.href);
    });
    if (filterHidden) {
      const indexOf = new Map(live.map((el, index) => [el, index]));
      const { gone, textless } = hiddenWithin(element);
      for (const el of textless) {
        for (const node of [...copied[indexOf.get(el)!].childNodes])
          if (node.nodeType === 3 /* TEXT_NODE */) node.remove();
      }
      for (const el of gone) copied[indexOf.get(el)!].remove();
    }
  }
  for (const dropped of copy.querySelectorAll(DROPPED)) dropped.remove();
  const comments: Comment[] = [];
  const walker = inert.createTreeWalker(copy, SHOW_COMMENT);
  for (let comment = walker.nextNode(); comment; comment = walker.nextNode()) comments.push(comment as Comment);
  for (const comment of comments) comment.remove();
  return copy.outerHTML;
}

/**
 * How the page hides an element: `gone` with all it holds, or `textless`, its own text hidden while
 * a child may show again (`visibility: visible` under `visibility: hidden`).
 */
function hiddenKind(element: Element): 'gone' | 'textless' | undefined {
  const style = getComputedStyle(element);
  if (style.display === 'none' || style.getPropertyValue('content-visibility') === 'hidden') return 'gone';
  if (style.visibility === 'hidden' || style.visibility === 'collapse') return 'textless';
  return undefined;
}

/** The hidden elements under `root`; nothing under a `gone` one is looked at. */
function hiddenWithin(root: Element): { gone: Element[]; textless: Element[] } {
  const gone: Element[] = [];
  const textless: Element[] = [];
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_ELEMENT, {
    acceptNode(node) {
      const kind = hiddenKind(node as Element);
      if (kind === 'gone') {
        gone.push(node as Element);
        return NodeFilter.FILTER_REJECT;
      }
      if (kind === 'textless') textless.push(node as Element);
      return NodeFilter.FILTER_ACCEPT;
    },
  });
  while (walker.nextNode());
  return { gone, textless };
}
