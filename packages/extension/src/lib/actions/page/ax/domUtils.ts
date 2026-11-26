/**
 * Copyright (c) Microsoft Corporation.
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 * http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

/*
 * Vendored from microsoft/playwright v1.65.1, packages/injected/src/domUtils.ts, for the
 * content-script accessibility tree of `page.snapshot` (docs/research/content-script-ax.md).
 * Modified by the vexdom authors (each change is marked `vexdom:` below): `shadowRootOf`,
 * `assignedSlotOf` and `openModalDialog` added, with their per-pass caches.
 */

type GlobalOptions = {
  browserNameForWorkarounds?: string;
};
let globalOptions: GlobalOptions = {};
export function setGlobalOptions(options: GlobalOptions) {
  globalOptions = options;
}

export function isInsideScope(scope: Node, element: Element | undefined): boolean {
  while (element) {
    if (scope.contains(element))
      return true;
    element = enclosingShadowHost(element);
  }
  return false;
}

export function enclosingElement(node: Node) {
  if (node.nodeType === 1 /* Node.ELEMENT_NODE */)
    return node as Element;
  return node.parentElement ?? undefined;
}

export function parentElementOrShadowHost(element: Element): Element | undefined {
  if (element.parentElement)
    return element.parentElement;
  if (!element.parentNode)
    return;
  if (element.parentNode.nodeType === 11 /* Node.DOCUMENT_FRAGMENT_NODE */ && (element.parentNode as ShadowRoot).host)
    return (element.parentNode as ShadowRoot).host;
}

// https://html.spec.whatwg.org/multipage/interaction.html#inert-subtrees
// Inert elements cannot be focused, clicked or otherwise interacted with.
export function isElementInert(element: Element): boolean {
  // Inert applies to all flat tree descendants, so we look up the hierarchy.
  for (let e: Element | undefined = element; e; e = parentElementOrShadowHost(e)) {
    if (e.hasAttribute('inert'))
      return true;
  }
  return false;
}

export function enclosingShadowRootOrDocument(element: Element): Document | ShadowRoot | undefined {
  let node: Node = element;
  while (node.parentNode)
    node = node.parentNode;
  if (node.nodeType === 11 /* Node.DOCUMENT_FRAGMENT_NODE */ || node.nodeType === 9 /* Node.DOCUMENT_NODE */)
    return node as Document | ShadowRoot;
}

// vexdom: the elements `attachShadow` accepts besides autonomous custom elements. Only they
// can host an author shadow root, so only they are worth a closed-root lookup.
const kShadowHostTagNames = new Set(['ARTICLE', 'ASIDE', 'BLOCKQUOTE', 'BODY', 'DIV', 'FOOTER', 'H1', 'H2', 'H3', 'H4', 'H5', 'H6', 'HEADER', 'MAIN', 'NAV', 'P', 'SECTION', 'SPAN']);

type ClosedRootApi = { dom?: { openOrClosedShadowRoot?: (element: HTMLElement) => ShadowRoot | null } };

// vexdom: the shadow root an element hosts, open or closed. A page cannot hide a closed root
// from an extension: Chrome lends content scripts `chrome.dom.openOrClosedShadowRoot`, Firefox
// gives them `Element.openOrClosedShadowRoot`.
export function shadowRootOf(element: Element): ShadowRoot | null {
  const open = element.shadowRoot;
  if (open)
    return open;
  const tagName = elementSafeTagName(element);
  if (element.namespaceURI !== 'http://www.w3.org/1999/xhtml' || (!tagName.includes('-') && !kShadowHostTagNames.has(tagName)))
    return null;
  const cached = cacheClosedShadowRoot?.get(element);
  if (cached !== undefined)
    return cached;
  const root = readClosedShadowRoot(element);
  cacheClosedShadowRoot?.set(element, root);
  return root;
}

function readClosedShadowRoot(element: Element): ShadowRoot | null {
  const firefox = (element as Element & { openOrClosedShadowRoot?: ShadowRoot | null }).openOrClosedShadowRoot;
  if (firefox !== undefined)
    return firefox ?? null;
  const read = (globalThis as { chrome?: ClosedRootApi }).chrome?.dom?.openOrClosedShadowRoot;
  if (!read)
    return null;
  try {
    return read(element as HTMLElement) ?? null;
  } catch {
    return null;
  }
}

// vexdom: the slot a light-tree node is assigned to. `assignedSlot` answers only for open
// shadow roots, so a closed root's slots are asked for the nodes they hold.
export function assignedSlotOf(node: Node): HTMLSlotElement | null {
  const slot = (node as Element | Text).assignedSlot;
  if (slot)
    return slot;
  const host = node.parentNode;
  if (!host || host.nodeType !== 1 /* Node.ELEMENT_NODE */)
    return null;
  const root = shadowRootOf(host as Element);
  if (!root || root.mode !== 'closed')
    return null;
  return closedSlotAssignments(root).get(node) ?? null;
}

function closedSlotAssignments(root: ShadowRoot): Map<Node, HTMLSlotElement> {
  let assignments = cacheClosedSlotAssignments?.get(root);
  if (!assignments) {
    assignments = new Map();
    for (const slot of root.querySelectorAll('slot')) {
      for (const assigned of slot.assignedNodes()) {
        if (!assignments.has(assigned))
          assignments.set(assigned, slot);
      }
    }
    cacheClosedSlotAssignments?.set(root, assignments);
  }
  return assignments;
}

// vexdom: the <dialog> opened with showModal(), which makes the rest of its document inert.
export function openModalDialog(document: Document): Element | null {
  const cached = cacheModalDialog?.get(document);
  if (cached !== undefined)
    return cached;
  let modal: Element | null = null;
  try {
    modal = document.querySelector('dialog:modal');
  } catch {
    // A browser without :modal has no modal dialogs to find.
  }
  cacheModalDialog?.set(document, modal);
  return modal;
}

function enclosingShadowHost(element: Element): Element | undefined {
  while (element.parentElement)
    element = element.parentElement;
  return parentElementOrShadowHost(element);
}

// Assumption: if scope is provided, element must be inside scope's subtree.
export function closestCrossShadow(element: Element | undefined, css: string, scope?: Document | Element): Element | undefined {
  while (element) {
    const closest = element.closest(css);
    if (scope && closest !== scope && closest?.contains(scope))
      return;
    if (closest)
      return closest;
    element = enclosingShadowHost(element);
  }
}

export function getElementComputedStyle(element: Element, pseudo?: string): CSSStyleDeclaration | undefined {
  const cache = pseudo === '::before' ? cacheStyleBefore : pseudo === '::after' ? cacheStyleAfter : cacheStyle;
  if (cache && cache.has(element))
    return cache.get(element);
  const style = element.ownerDocument && element.ownerDocument.defaultView ? element.ownerDocument.defaultView.getComputedStyle(element, pseudo) : undefined;
  cache?.set(element, style);
  return style;
}

export function isElementStyleVisibilityVisible(element: Element, style?: CSSStyleDeclaration): boolean {
  const cached = cacheStyleVisibility?.get(element);
  if (cached !== undefined)
    return cached;
  const result = computeElementStyleVisibilityVisible(element, style);
  cacheStyleVisibility?.set(element, result);
  return result;
}

function computeElementStyleVisibilityVisible(element: Element, style?: CSSStyleDeclaration): boolean {
  style = style ?? getElementComputedStyle(element);
  if (!style)
    return true;
  // Element.checkVisibility checks for content-visibility and also looks at
  // styles up the flat tree including user-agent ShadowRoots, such as the
  // details element for example.
  if (!element.checkVisibility() && !isWebKitListBoxOptionVisible(element, style))
    return false;
  if (style.visibility !== 'visible')
    return false;
  return true;
}

export function isListBoxSelect(select: HTMLSelectElement): boolean {
  return select.multiple || select.size > 1;
}

// WebKit does not create renderers for list box options, so checkVisibility() returns false for them.
function isWebKitListBoxOptionVisible(element: Element, style: CSSStyleDeclaration): boolean {
  if (globalOptions.browserNameForWorkarounds !== 'webkit' || element.nodeName !== 'OPTION' || style.display === 'none')
    return false;
  const select = element.closest('select');
  if (!select || !isListBoxSelect(select))
    return false;
  for (let e = element.parentElement; e && e !== select; e = e.parentElement) {
    if (getElementComputedStyle(e)?.display === 'none')
      return false;
  }
  return isElementStyleVisibilityVisible(select);
}

export function computeBox(element: Element) {
  // Note: this logic should be similar to waitForDisplayedAtStablePosition() to avoid surprises.
  const style = getElementComputedStyle(element);
  if (!style)
    return { visible: true, inline: false };
  const cursor = style.cursor;
  if (style.display === 'contents') {
    // display:contents is not rendered itself, but its child nodes are.
    for (let child = element.firstChild; child; child = child.nextSibling) {
      if (child.nodeType === 1 /* Node.ELEMENT_NODE */ && isElementVisible(child as Element))
        return { visible: true, inline: false, cursor };
      if (child.nodeType === 3 /* Node.TEXT_NODE */ && isVisibleTextNode(child as Text))
        return { visible: true, inline: true, cursor };
    }
    return { visible: false, inline: false, cursor };
  }
  if (!isElementStyleVisibilityVisible(element, style))
    return { cursor, visible: false, inline: false };
  const rect = element.getBoundingClientRect();
  return { cursor, visible: rect.width > 0 && rect.height > 0, inline: style.display === 'inline' };
}

export function isElementVisible(element: Element): boolean {
  return computeBox(element).visible;
}

const kTextNodeRange = Symbol('playwrightTextNodeRange');

export function isVisibleTextNode(node: Text) {
  // https://stackoverflow.com/questions/1461059/is-there-an-equivalent-to-getboundingclientrect-for-text-nodes
  // Reuse a single range per document. Browsers update every live range upon each DOM mutation
  // until the range is garbage collected, so creating a range per text node slows the page down.
  const document = node.ownerDocument as Document & { [kTextNodeRange]?: Range };
  const range = (document[kTextNodeRange] ??= document.createRange());
  range.selectNode(node);
  const rect = range.getBoundingClientRect();
  const result = rect.width > 0 && rect.height > 0;
  // Collapse the range back to the document, so that it does not retain the node.
  range.setStart(document, 0);
  range.collapse(true);
  return result;
}

export function elementSafeTagName(element: Element) {
  const tagName = element.tagName;
  if (typeof tagName === 'string') {  // Fast path.
    // Tag names in html documents are already uppercase. Lowercase names come from
    // svg/mathml elements and from xml/xhtml documents, and they all start with
    // a lowercase letter, so uppercasing can be skipped otherwise.
    const firstCharCode = tagName.charCodeAt(0);
    if (firstCharCode >= 97 && firstCharCode <= 122)
      return tagName.toUpperCase();
    return tagName;
  }
  // Named inputs, e.g. <input name=tagName>, will be exposed as fields on the parent <form>
  // and override its properties.
  if (element instanceof HTMLFormElement)
    return 'FORM';
  // Elements from the svg namespace do not have uppercase tagName right away.
  return element.tagName.toUpperCase();
}

let cacheStyle: Map<Element, CSSStyleDeclaration | undefined> | undefined;
let cacheStyleBefore: Map<Element, CSSStyleDeclaration | undefined> | undefined;
let cacheStyleAfter: Map<Element, CSSStyleDeclaration | undefined> | undefined;
let cacheStyleVisibility: Map<Element, boolean> | undefined;
// vexdom: closed-root lookups, closed slot assignments and modal dialogs, per pass.
let cacheClosedShadowRoot: Map<Element, ShadowRoot | null> | undefined;
let cacheClosedSlotAssignments: Map<ShadowRoot, Map<Node, HTMLSlotElement>> | undefined;
let cacheModalDialog: Map<Document, Element | null> | undefined;
let cachesCounter = 0;

export function beginDOMCaches() {
  ++cachesCounter;
  cacheStyle ??= new Map();
  cacheStyleBefore ??= new Map();
  cacheStyleAfter ??= new Map();
  cacheStyleVisibility ??= new Map();
  cacheClosedShadowRoot ??= new Map();
  cacheClosedSlotAssignments ??= new Map();
  cacheModalDialog ??= new Map();
}

export function endDOMCaches() {
  if (!--cachesCounter) {
    cacheStyle = undefined;
    cacheStyleBefore = undefined;
    cacheStyleAfter = undefined;
    cacheStyleVisibility = undefined;
    cacheClosedShadowRoot = undefined;
    cacheClosedSlotAssignments = undefined;
    cacheModalDialog = undefined;
  }
}
