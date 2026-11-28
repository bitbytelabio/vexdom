/**
 * `page.snapshot`: the document as a compact accessibility tree for the Vexdom Bridge
 * (docs/research/content-script-ax.md §6-7). The Bridge expands it into agent-browser's AX nodes
 * (crates/vexdom-snapshot/src/wire.rs) and renders the outline agents act on by ref.
 *
 * Every element that can get a ref carries an id from this document's table, so the same element
 * keeps its id — and its ref — across snapshots, and `docKey` tells this document's ids from those
 * of a document that is gone. Nothing is written to the page.
 */

import { z } from 'zod';
import { isOverlay } from '@/lib/overlay';
import { ActionError, defineAction } from '../core';
import { announceFrame, mintKey } from '@/lib/frames/probe';
import {
  assignedSlotOf,
  elementSafeTagName,
  getElementComputedStyle,
  isInsideScope,
  openModalDialog,
  parentElementOrShadowHost,
  shadowRootOf,
} from './ax/domUtils';
import {
  beginAriaCaches,
  endAriaCaches,
  getAriaChecked,
  getAriaDisabled,
  getAriaExpanded,
  getAriaLevel,
  getAriaRole,
  getAriaSelected,
  getElementAccessibleName,
  isElementIgnoredForAria,
} from './ax/roleUtils';

/** States the Bridge prints; `level`, `checked` and `expanded` print even when false. */
export interface WireStates {
  level?: number;
  checked?: 'true' | 'false' | 'mixed';
  expanded?: boolean;
  selected?: boolean;
  disabled?: boolean;
  required?: boolean;
  focused?: boolean;
}

/** Why an element without an interactive role acts like a control, in agent-browser's field names. */
export interface CursorInfo {
  hasCursorPointer: boolean;
  hasOnClick: boolean;
  hasTabIndex: boolean;
  isEditable: boolean;
  text: string;
  isNative: boolean;
  hiddenInputType: string | null;
  hiddenInputChecked: string | null;
}

export interface WireNode {
  /** Durable element id; only an element that can get a ref has one. */
  id?: number;
  /** ARIA role, or Chrome's `RootWebArea`, `StaticText` and `Iframe` where the Bridge keys on them. */
  r: string;
  n?: string;
  v?: string | number;
  s?: WireStates;
  u?: string;
  h?: string;
  k?: CursorInfo;
  sh?: true;
  c?: number[];
}

/** A child frame not walked inline: the background fills in its id and tree. */
export interface FrameSlot {
  node: number;
  frameId?: number;
  probe?: string;
  title?: string;
  name?: string;
  sameOrigin: boolean;
  snapshot?: PageSnapshot;
}

export interface PageSnapshot {
  v: 1;
  docKey: string;
  url: string;
  title: string;
  viewport: { w: number; h: number; scrollX: number; scrollY: number };
  nodes: WireNode[];
  frames: FrameSlot[];
  /** With `viewport`: the page regions left out because they lay wholly off screen, by direction. */
  offscreen?: Offscreen;
  stats: { elements: number; nodes: number; ms: number; truncated: boolean };
}

export interface Offscreen {
  above: number;
  below: number;
  aside: number;
}

/** A ref as the Bridge hands it back: the document and id a snapshot gave out, with the role and name it showed. */
export interface ElementRef {
  doc: string;
  id: number;
  role?: string;
  name?: string;
  nth?: number;
}

const snapshotInput = z.object({
  selector: z.string().optional().describe('CSS selector of the subtrees to walk; the whole page when omitted'),
  interactive: z.boolean().default(false).describe('Only the elements that can get a ref'),
  maxNodes: z.number().int().positive().default(5000).describe('Stop after this many nodes'),
  includeFrames: z
    .boolean()
    .default(true)
    .describe('Walk blank same-origin frames inline and list the other frames for the extension to fill in'),
  viewport: z.boolean().default(false).describe('Only elements on screen; the reply says what lies beyond'),
  settleMs: z
    .number()
    .int()
    .min(0)
    .max(2_000)
    .default(0)
    .describe('First wait until the page has made no DOM change for 50 ms, at most this long'),
});

export const snapshot = defineAction({
  name: 'page.snapshot',
  description:
    'The page as an accessibility tree, for the Vexdom Bridge to render as the outline agents act on by ref.',
  input: snapshotInput,
  // Synchronous without a settle, so callers that walk a static page need not await it.
  execute(input) {
    return input.settleMs > 0 ? domQuiet(input.settleMs).then(() => takeSnapshot(input)) : takeSnapshot(input);
  },
});

/** How long the DOM must stay unchanged before a settled snapshot is taken. */
const QUIET_MS = 50;

/** Resolves once the document has gone `QUIET_MS` without a mutation, or after `maxMs`. */
function domQuiet(maxMs: number): Promise<void> {
  return new Promise((resolve) => {
    let quiet: ReturnType<typeof setTimeout> | undefined;
    const done = () => {
      observer.disconnect();
      clearTimeout(quiet);
      clearTimeout(cap);
      resolve();
    };
    const observer = new MutationObserver(() => {
      clearTimeout(quiet);
      quiet = setTimeout(done, QUIET_MS);
    });
    observer.observe(document, { subtree: true, childList: true, attributes: true, characterData: true });
    quiet = setTimeout(done, QUIET_MS);
    const cap = setTimeout(done, maxMs);
  });
}

function takeSnapshot(input: z.output<typeof snapshotInput>): PageSnapshot {
  const started = performance.now();
  const docKey = documentKey();
  const { interactive, maxNodes, includeFrames, viewport } = input;
  const walked = withAriaCaches(() => {
    const result = walk(rootsFor(input.selector), { interactive, maxNodes, includeFrames, viewport, assign: true });
    addHints(result);
    rememberAnchors(result);
    return result;
  });
  forgetCollected();
  return {
    v: 1,
    docKey,
    url: location.href.toWellFormed(),
    title: document.title.toWellFormed(),
    viewport: { w: innerWidth, h: innerHeight, scrollX: Math.round(scrollX), scrollY: Math.round(scrollY) },
    nodes: walked.nodes,
    frames: walked.frames,
    ...(viewport ? { offscreen: walked.offscreen } : {}),
    stats: {
      elements: walked.elements,
      nodes: walked.nodes.length,
      ms: Math.round((performance.now() - started) * 10) / 10,
      truncated: walked.truncated,
    },
  };
}

// ---------------------------------------------------------------------------------------------
// The document's ids. They live as long as this content script, one per document.

let docKey: string | undefined;
const idOfElement = new WeakMap<Element, number>();
const elementOfId = new Map<number, WeakRef<Element>>();
/** Ids of elements seen in a shadow tree: recovered only within their anchor. */
const shadowIds = new Set<number>();

/** Where a ref that needed nth, or lived in a shadow tree, can be looked for again. */
interface Anchor {
  /** The nearest ancestor (crossing shadow roots to their hosts) holding no other element with this role and name. */
  scope: WeakRef<Element>;
  /** The nearest such ancestor with a key unique in its root: found again when it is re-rendered. */
  keyed?: { attr: 'id' | 'data-testid'; value: string; root: WeakRef<Document | ShadowRoot> };
}
const anchorOfId = new Map<number, Anchor>();
let nextId = 1;

/** The key of this document's refs; a ref under any other key was issued for a document that is gone. */
export function documentKey(): string {
  return (docKey ??= mintKey());
}

/** The live element this document gave `id`; null for another document's ref or one that is gone. */
export function elementForId(doc: string, id: number): Element | null {
  if (doc !== documentKey()) return null;
  const element = elementOfId.get(id)?.deref();
  return element?.isConnected ? element : null;
}

function idOf(element: Element, inShadow: boolean): number {
  let id = idOfElement.get(element);
  if (id === undefined) {
    id = nextId++;
    idOfElement.set(element, id);
    elementOfId.set(id, new WeakRef(element));
  }
  if (inShadow) shadowIds.add(id);
  return id;
}

function forgetCollected(): void {
  for (const [id, element] of elementOfId) if (!element.deref()) elementOfId.delete(id);
  for (const id of anchorOfId.keys()) if (!elementOfId.has(id)) anchorOfId.delete(id);
}

/** The parent in the composed tree: a shadow root's child goes up to its host. */
function composedParent(element: Element): Element | null {
  if (element.parentElement) return element.parentElement;
  const parent = element.parentNode;
  return parent?.nodeType === Node.DOCUMENT_FRAGMENT_NODE ? ((parent as ShadowRoot).host ?? null) : null;
}

/** The elements of `root` whose `attr` is `value`. */
function keyedIn(root: Document | ShadowRoot, attr: string, value: string): NodeListOf<Element> {
  return root.querySelectorAll(`[${attr}="${value.replace(/["\\]/g, '\\$&')}"]`);
}

/** `ancestor`'s `id`, else its `data-testid`, when no other element in its root has it. */
function uniqueKey(ancestor: Element): Anchor['keyed'] {
  const root = ancestor.getRootNode() as Document | ShadowRoot;
  if (typeof root.querySelectorAll !== 'function') return undefined;
  for (const attr of ['id', 'data-testid'] as const) {
    const value = ancestor.getAttribute(attr);
    if (value && keyedIn(root, attr, value).length === 1) return { attr, value, root: new WeakRef(root) };
  }
  return undefined;
}

/**
 * Remembers an anchor for each ref that shares its role and name or lives in a shadow tree: the
 * nearest ancestor holding no other element like it, and the nearest such one with a unique key.
 */
function rememberAnchors({ nodes, refable }: Walked): void {
  const groups = new Map<string, { id: number; element: Element }[]>();
  for (const { index, element } of refable) {
    const id = nodes[index].id;
    if (id === undefined) continue;
    const { role, name } = asRendered(nodes[index]);
    const key = `${role}\n${name}`;
    const group = groups.get(key);
    if (group) group.push({ id, element });
    else groups.set(key, [{ id, element }]);
  }
  for (const members of groups.values()) {
    const needsAnchor = (id: number) => members.length > 1 || shadowIds.has(id);
    // A ref that stopped being shared drops its old anchor.
    for (const { id } of members) if (!needsAnchor(id)) anchorOfId.delete(id);
    if (!members.some(({ id }) => needsAnchor(id))) continue;
    const chains = members.map(({ element }) => {
      const chain: Element[] = [];
      for (let at = composedParent(element); at; at = composedParent(at)) chain.push(at);
      return chain;
    });
    const held = new Map<Element, number>();
    for (const chain of chains) for (const ancestor of chain) held.set(ancestor, (held.get(ancestor) ?? 0) + 1);
    members.forEach(({ id }, at) => {
      if (!needsAnchor(id)) return;
      const own = chains[at].filter((ancestor) => held.get(ancestor) === 1);
      if (own.length === 0) {
        anchorOfId.delete(id);
        return;
      }
      let keyed: Anchor['keyed'];
      for (const ancestor of own) if ((keyed = uniqueKey(ancestor))) break;
      anchorOfId.set(id, { scope: new WeakRef(own[0]), ...(keyed ? { keyed } : {}) });
    });
  }
}

const DOCUMENT_GONE =
  'Ref is from a document that is no longer loaded (the page navigated or the frame was removed). Take a new snapshot and use a fresh ref.';

const STALE = 'Element ref is stale. Take a new snapshot.';

/**
 * The element a ref names. A live element is itself — `isConnected`, since a `WeakRef` can still
 * hand back a detached one. A replaced element is found again by its role and name: within the
 * ancestor that held only it when the ref was given (for a ref that needed nth or lived in a
 * shadow tree), else in the whole document; never when two match.
 */
export function resolveRef(ref: ElementRef): Element {
  if (ref.doc !== documentKey()) throw new ActionError(DOCUMENT_GONE, 'STALE_REF');
  const known = elementOfId.get(ref.id)?.deref();
  if (known?.isConnected) return known;
  const found = recover(ref);
  if (found) return found;
  throw new ActionError(STALE, 'STALE_REF');
}

function recover(ref: ElementRef): Element | null {
  if (ref.role === undefined || ref.name === undefined) return null;
  const anchor = anchorOfId.get(ref.id);
  if (anchor) return recoverIn(anchor, ref);
  if (ref.nth !== undefined || shadowIds.has(ref.id)) return null;
  const matches = matchesIn(document.body ?? document.documentElement, ref);
  return matches.length === 1 ? matches[0] : null;
}

/** The anchor's scope, else its keyed ancestor found again: the first with a match decides. */
function recoverIn(anchor: Anchor, ref: ElementRef): Element | null {
  const scopes: Element[] = [];
  const scope = anchor.scope.deref();
  if (scope?.isConnected) scopes.push(scope);
  const root = anchor.keyed?.root.deref();
  if (
    anchor.keyed &&
    root &&
    (root === document || (root.nodeType === Node.DOCUMENT_FRAGMENT_NODE && (root as ShadowRoot).host?.isConnected))
  ) {
    const keyed = keyedIn(root, anchor.keyed.attr, anchor.keyed.value);
    if (keyed.length === 1 && !scopes.includes(keyed[0])) scopes.push(keyed[0]);
  }
  for (const within of scopes) {
    const matches = matchesIn(within, ref);
    if (matches.length === 1) return matches[0];
    if (matches.length > 1) return null;
  }
  return null;
}

/** The elements under `root` the Bridge would render with the ref's role and name. */
function matchesIn(root: Element, ref: ElementRef): Element[] {
  const { nodes, refable } = withAriaCaches(() =>
    walk([root], { interactive: true, maxNodes: Infinity, includeFrames: true, assign: false }),
  );
  return refable
    .filter(({ index }) => {
      const { role, name } = asRendered(nodes[index]);
      return role === ref.role && name === ref.name;
    })
    .map(({ element }) => element);
}

// ---------------------------------------------------------------------------------------------
// The walk.

/** The Bridge refs these roles always (crates/vexdom-snapshot/src/snapshot.rs). */
const INTERACTIVE_ROLES = new Set([
  'button',
  'link',
  'textbox',
  'checkbox',
  'radio',
  'combobox',
  'listbox',
  'menuitem',
  'menuitemcheckbox',
  'menuitemradio',
  'option',
  'searchbox',
  'slider',
  'spinbutton',
  'switch',
  'tab',
  'treeitem',
  'Iframe',
]);
const TABLE_CELL_ROLES = new Set(['cell', 'gridcell', 'columnheader', 'rowheader']);
/** Refed only when named. */
const CONTENT_ROLES = new Set(['heading', 'listitem', 'article', 'region', 'main', 'navigation']);
/** ARIA "Children Presentational": the descendants are not exposed, only their text. */
const PRESENTATIONAL_CHILDREN = new Set([
  'button',
  'checkbox',
  'img',
  'menuitemcheckbox',
  'menuitemradio',
  'meter',
  'option',
  'progressbar',
  'radio',
  'scrollbar',
  'separator',
  'slider',
  'switch',
  'tab',
]);
const CHECKED_ROLES = new Set(['checkbox', 'radio', 'switch', 'menuitemcheckbox', 'menuitemradio']);
const LEVEL_ROLES = new Set(['heading', 'listitem', 'row', 'treeitem']);
const RANGE_ROLES = new Set(['slider', 'spinbutton', 'progressbar', 'meter', 'scrollbar']);
const NATIVE_TAGS = new Set(['A', 'BUTTON', 'INPUT', 'SELECT', 'TEXTAREA', 'DETAILS', 'SUMMARY']);
/** Inputs whose value is a label or a secret, not text someone typed. */
const VALUELESS_INPUTS = new Set([
  'button',
  'submit',
  'reset',
  'image',
  'checkbox',
  'radio',
  'file',
  'hidden',
  'password',
]);
const CONTROLS =
  'a[href], button, input:not([type="hidden"]), select, textarea, summary, [role="button"], [role="link"], ' +
  '[role="checkbox"], [role="radio"], [role="tab"], [role="menuitem"], [role="switch"], [role="textbox"], [role="combobox"]';

/** Values are printed raw on one line, so they are flattened and capped (in code points). */
const MAX_VALUE = 200;
const MAX_CURSOR_TEXT = 100;
const MAX_URL = 2048;
/** The Bridge renders child frames at most this deep. */
const MAX_FRAME_DEPTH = 4;

interface WalkOptions {
  interactive: boolean;
  maxNodes: number;
  includeFrames: boolean;
  /** Skip the subtrees of this document that lie wholly off screen, counting them. */
  viewport?: boolean;
  /** Give the elements that can get a ref their ids; ref recovery only looks. */
  assign: boolean;
}

/** An emitted node that text runs and child nodes attach to. */
interface Container {
  index: number;
  text: string[];
}

/** The flat-tree children of one element still to visit. */
interface Visit {
  children: readonly Node[];
  at: number;
  into: Container;
  /** Text directly in this element renders: its `visibility` is not hidden. */
  text: boolean;
  /** Inside an element whose children are presentational: only text counts. */
  textOnly: boolean;
  /** Frames deep: a blank frame walked inline is one deeper than its parent. */
  depth: number;
  done?: () => void;
}

interface Walked {
  nodes: WireNode[];
  frames: FrameSlot[];
  /** The nodes that can get a ref, with their element. */
  refable: { index: number; element: Element }[];
  elements: number;
  truncated: boolean;
  offscreen: Offscreen;
}

/**
 * Depth first over the flat tree (shadow roots, slots, `aria-owns`), with an explicit stack: a
 * page can nest deeper than a recursive walk's stack allows. Text runs between two nodes become
 * one `StaticText`, as the Bridge would merge them anyway.
 */
function walk(roots: readonly Element[], options: WalkOptions): Walked {
  const nodes: WireNode[] = [];
  const frames: FrameSlot[] = [];
  const refable: Walked['refable'] = [];
  const visited = new Set<Element>();
  const stack: Visit[] = [];
  const texts = !options.interactive;
  const focused = activeElement(document);
  const offscreen: Offscreen = { above: 0, below: 0, aside: 0 };
  let elements = 0;
  let truncated = false;

  const add = (node: WireNode): number => {
    if (nodes.length >= options.maxNodes) {
      truncated = true;
      return -1;
    }
    return nodes.push(node) - 1;
  };

  const attach = (into: Container, index: number): void => {
    (nodes[into.index].c ??= []).push(index);
  };

  const flush = (into: Container, last = false): void => {
    if (!into.text.length) return;
    const text = flat(into.text.join(''));
    into.text.length = 0;
    const parent = nodes[into.index];
    // The Bridge drops a lone text child that repeats its parent's name, so it is not sent.
    if (!text || (last && !parent.c && text === parent.n)) return;
    const index = add({ r: 'StaticText', n: text });
    if (index >= 0) attach(into, index);
  };

  /** Walks an element that shows no node of its own: its content goes to the nearest one that does. */
  const pass = (from: Visit, text: boolean, block: boolean, children: readonly Node[]): void => {
    const into = from.into;
    const gap = texts && block;
    if (gap) into.text.push(' ');
    stack.push({
      children,
      at: 0,
      into,
      text,
      textOnly: from.textOnly,
      depth: from.depth,
      done: gap ? () => into.text.push(' ') : undefined,
    });
  };

  const descend = (into: Container, children: readonly Node[], textOnly: boolean, depth: number): void => {
    stack.push({ children, at: 0, into, text: true, textOnly, depth, done: () => flush(into, true) });
  };

  const frame = (element: Element, into: Container, depth: number): void => {
    if (!options.includeFrames) return;
    const doc = contentDocumentOf(element);
    if (doc?.body && depth < MAX_FRAME_DEPTH && isBlankFrame(element, doc)) {
      // No content script of its own runs in it, so its elements are this document's to name.
      descend(into, [doc.body], false, depth + 1);
      return;
    }
    const title = flat(element.getAttribute('title') ?? '');
    const name = flat(element.getAttribute('name') ?? '');
    // The background names the frame from the nonce (`claimFrame`) and fills in `frameId`.
    const probe = options.assign ? announceFrame(element) : undefined;
    frames.push({
      node: into.index,
      ...(probe ? { probe } : {}),
      ...(title ? { title } : {}),
      ...(name ? { name } : {}),
      sameOrigin: !!doc,
    });
  };

  const enter = (element: Element, from: Visit): void => {
    if (visited.has(element)) return;
    visited.add(element);
    elements++;
    const tag = elementSafeTagName(element);
    if (tag === 'BR') {
      if (texts && from.text) from.into.text.push('\n');
      return;
    }
    if (isElementIgnoredForAria(element) || isOverlay(element)) return;
    const style = getElementComputedStyle(element);
    if (!style || excluded(element, tag, style)) return;
    // `visibility: hidden` hides the element and its text, but a child can be visible again.
    const shown = style.visibility !== 'hidden' && style.visibility !== 'collapse';
    // The body's own box can end above content that overflows it, so only its children are judged.
    if (options.viewport && from.depth === 0 && tag !== 'BODY' && tag !== 'HTML') {
      const side = offscreenSide(element, style);
      if (side) {
        offscreen[side]++;
        return;
      }
    }
    const block = isBlock(style);
    if (tag === 'BODY' || tag === 'HTML' || tag === 'SLOT' || from.textOnly || !shown) {
      pass(from, shown, block, flatChildren(element, tag));
      return;
    }
    const role = roleOf(element, tag);
    if (role === 'none' || role === 'presentation') {
      pass(from, true, block, flatChildren(element, tag));
      return;
    }

    const cursor = INTERACTIVE_ROLES.has(role) ? undefined : cursorOf(element, tag, style);
    const named = texts || !!cursor || INTERACTIVE_ROLES.has(role) || CONTENT_ROLES.has(role);
    const name = named ? flat(getElementAccessibleName(element, false).text) : '';
    // Interactive mode refs no table cell (the Bridge's rule): the controls inside stand for it.
    const canRef =
      INTERACTIVE_ROLES.has(role) ||
      (texts && TABLE_CELL_ROLES.has(role)) ||
      (CONTENT_ROLES.has(role) && !!name) ||
      !!cursor;
    const children = flatChildren(element, tag);
    if (!canRef && (!texts || (role === 'generic' && isBare(children, style)))) {
      pass(from, true, block, children);
      return;
    }

    flush(from.into);
    const inShadow = isInShadowTree(element);
    const node: WireNode = canRef && options.assign ? { id: idOf(element, inShadow), r: role } : { r: role };
    if (name) node.n = name;
    const value = valueOf(element, tag, role);
    if (value !== undefined) node.v = value;
    const states = statesOf(element, tag, role, focused);
    if (states) node.s = states;
    if (role === 'link') {
      const url = hrefOf(element);
      if (url) node.u = url;
    }
    if (cursor) node.k = cursor;
    if (inShadow) node.sh = true;
    const index = add(node);
    if (index < 0) return;
    attach(from.into, index);
    if (canRef) refable.push({ index, element });

    const into: Container = { index, text: [] };
    if (role === 'Iframe') {
      frame(element, into, from.depth);
      return;
    }
    // A text field's text is its value.
    if (role === 'textbox' || role === 'searchbox' || isEditingHost(element)) return;
    const textOnly = PRESENTATIONAL_CHILDREN.has(role);
    if (texts || !textOnly) descend(into, children, textOnly, from.depth);
  };

  const title = flat(document.title);
  nodes.push(title ? { r: 'RootWebArea', n: title } : { r: 'RootWebArea' });
  const root: Container = { index: 0, text: [] };
  stack.push({
    children: roots,
    at: 0,
    into: root,
    text: true,
    textOnly: false,
    depth: 0,
    done: () => flush(root, true),
  });
  while (stack.length && !truncated) {
    const visit = stack[stack.length - 1];
    if (visit.at === visit.children.length) {
      stack.pop();
      visit.done?.();
      continue;
    }
    const child = visit.children[visit.at++];
    if (child.nodeType === 3 /* Node.TEXT_NODE */) {
      if (texts && visit.text) visit.into.text.push((child as Text).data);
    } else if (child.nodeType === 1 /* Node.ELEMENT_NODE */) {
      enter(child as Element, visit);
    }
  }
  return { nodes, frames, refable, elements, truncated, offscreen };
}

/**
 * Where an element lies when it is wholly outside the viewport; undefined when any of it shows,
 * when it is fixed (it moves with the viewport) or has no box of its own.
 */
function offscreenSide(element: Element, style: CSSStyleDeclaration): keyof Offscreen | undefined {
  if (style.position === 'fixed') return undefined;
  const rect = element.getBoundingClientRect();
  if (rect.width <= 0 || rect.height <= 0) return undefined;
  if (rect.bottom <= 0) return 'above';
  if (rect.top >= innerHeight) return 'below';
  if (rect.right <= 0 || rect.left >= innerWidth) return 'aside';
  return undefined;
}

/**
 * The selector's outermost matches, or the body. A match is skipped when an ancestor already
 * keeps it out of the tree, as the walk itself would have.
 */
function rootsFor(selector: string | undefined): Element[] {
  if (!selector) return [document.body ?? document.documentElement];
  let matches: Element[];
  try {
    matches = [...document.querySelectorAll(selector)];
  } catch {
    throw new ActionError(`${JSON.stringify(selector)} is not a valid CSS selector`, 'INVALID_TARGET');
  }
  if (!matches.length)
    throw new ActionError(`No element matches the selector ${JSON.stringify(selector)}`, 'TARGET_NOT_FOUND');
  let last: Element | undefined;
  const outermost = matches.filter((element) => {
    if (last?.contains(element)) return false;
    last = element;
    return true;
  });
  return outermost.filter((element) => !hiddenByAncestors(element));
}

function hiddenByAncestors(element: Element): boolean {
  for (let node: Element | undefined = element; node; node = parentElementOrShadowHost(node)) {
    const parent = node.parentElement;
    // A light child of a shadow host renders only through a slot.
    if (parent && shadowRootOf(parent) && !assignedSlotOf(node)) return true;
    if (node === element) continue;
    const style = getElementComputedStyle(node);
    if (isOverlay(node) || !style || excluded(node, elementSafeTagName(node), style)) return true;
  }
  return false;
}

/**
 * Out of the tree with its whole subtree: `display: none`, `aria-hidden`, `inert` or outside an
 * open modal dialog, and no box at all — closed <details> content, `content-visibility: hidden`.
 */
function excluded(element: Element, tag: string, style: CSSStyleDeclaration): boolean {
  if (style.display === 'none') return true;
  if (element.getAttribute('aria-hidden')?.toLowerCase() === 'true') return true;
  if (element.hasAttribute('inert') || blockedByModal(element)) return true;
  // display: contents has no box of its own; nor has a closed <select> for its options.
  if (style.display === 'contents' || tag === 'SLOT' || inSelect(element, tag)) return false;
  return !element.checkVisibility();
}

function blockedByModal(element: Element): boolean {
  const modal = openModalDialog(element.ownerDocument);
  return !!modal && !isInsideScope(modal, element) && !isInsideScope(element, modal);
}

const inSelect = (element: Element, tag: string): boolean =>
  (tag === 'OPTION' || tag === 'OPTGROUP') && !!element.closest('select');

const isBlock = (style: CSSStyleDeclaration): boolean =>
  !!style.display && style.display !== 'inline' && style.display !== 'contents';

/** A generic element with nothing of its own to show: empty, or an inline wrapper around one text. */
const isBare = (children: readonly Node[], style: CSSStyleDeclaration): boolean =>
  !children.length || (!isBlock(style) && children.length === 1 && children[0].nodeType === 3);

const roleOf = (element: Element, tag: string): string =>
  tag === 'IFRAME' || tag === 'FRAME' ? 'Iframe' : (getAriaRole(element) ?? 'generic');

/** Children in the flat tree: a shadow root's in place of the light ones, a slot's assigned nodes, then `aria-owns`. */
function flatChildren(element: Element, tag: string): Node[] {
  if (tag === 'INPUT' || tag === 'TEXTAREA') return [];
  if (tag === 'DETAILS' && !(element as HTMLDetailsElement).open) {
    const summary = element.querySelector(':scope > summary');
    return summary ? [summary] : [];
  }
  let children: Node[];
  if (tag === 'SLOT') {
    const assigned = (element as HTMLSlotElement).assignedNodes();
    children = assigned.length ? assigned : [...element.childNodes];
  } else {
    children = [...(shadowRootOf(element) ?? element).childNodes];
  }
  const owns = element.getAttribute('aria-owns');
  if (owns) {
    const root = element.getRootNode() as Document | ShadowRoot;
    for (const id of owns.split(/\s+/)) {
      const owned = id && typeof root.getElementById === 'function' ? root.getElementById(id) : null;
      if (owned) children.push(owned);
    }
  }
  return children;
}

function contentDocumentOf(element: Element): Document | null {
  try {
    return (element as HTMLIFrameElement).contentDocument;
  } catch {
    return null;
  }
}

/** about:blank or srcdoc, which the page itself fills and no content script of ours runs in. */
function isBlankFrame(element: Element, doc: Document): boolean {
  if (doc.URL === 'about:srcdoc') return true;
  const src = element.getAttribute('src')?.trim();
  return doc.URL === 'about:blank' && (!src || src === 'about:blank');
}

function isInShadowTree(element: Element): boolean {
  const root = element.getRootNode();
  return root.nodeType === 11 /* Node.DOCUMENT_FRAGMENT_NODE */ && !!(root as ShadowRoot).host;
}

function activeElement(doc: Document): Element | null {
  let active = doc.activeElement;
  for (let inner = active && shadowRootOf(active)?.activeElement; inner; inner = shadowRootOf(inner)?.activeElement) {
    active = inner;
  }
  return active;
}

/**
 * agent-browser's cursor scan (cli/src/native/snapshot.rs, the CDP scanner), on the style the walk
 * already read: `cursor: pointer` of its own, an `onclick`, a `tabindex` or `contenteditable`, on
 * an element with a box. Chrome's isolated world sees only the `onclick` attribute, not a handler
 * the page assigned; Firefox sees both.
 */
function cursorOf(element: Element, tag: string, style: CSSStyleDeclaration): CursorInfo | undefined {
  if (tag === 'DETAILS') return undefined;
  const hasCursorPointer = style.cursor === 'pointer';
  const hasOnClick = element.hasAttribute('onclick') || (element as HTMLElement).onclick != null;
  const tabIndex = element.getAttribute('tabindex');
  const hasTabIndex = tabIndex !== null && tabIndex !== '-1';
  const editable = element.getAttribute('contenteditable')?.toLowerCase();
  const isEditable = editable === '' || editable === 'true' || editable === 'plaintext-only';
  if (!hasCursorPointer && !hasOnClick && !hasTabIndex && !isEditable) return undefined;
  if (hasCursorPointer && !hasOnClick && !hasTabIndex && !isEditable) {
    // Only inherited from a clickable ancestor.
    const parent = assignedSlotOf(element) ?? parentElementOrShadowHost(element);
    if (parent && getElementComputedStyle(parent)?.cursor === 'pointer') return undefined;
  }
  // A link without href that wraps a control is covered by that control.
  if (tag === 'A' && [...element.querySelectorAll(CONTROLS)].some(isRendered)) return undefined;
  if (!hasBox(element, style)) return undefined;

  let hiddenInputType: string | null = null;
  let hiddenInputChecked: string | null = null;
  const input = element.querySelector<HTMLInputElement>('input[type="radio"], input[type="checkbox"]');
  if (input) {
    // A label styled as the control around a hidden radio or checkbox; the Bridge shows it as one.
    const inputStyle = getElementComputedStyle(input);
    if (inputStyle?.display === 'none' || inputStyle?.visibility === 'hidden' || input.hidden) {
      hiddenInputType = input.type;
      hiddenInputChecked = input.indeterminate ? 'mixed' : String(input.checked);
    }
  }
  return {
    hasCursorPointer,
    hasOnClick,
    hasTabIndex,
    isEditable,
    text: cap(flat(element.textContent ?? ''), MAX_CURSOR_TEXT),
    isNative: NATIVE_TAGS.has(tag),
    hiddenInputType,
    hiddenInputChecked,
  };
}

function isRendered(element: Element): boolean {
  if (element.closest('[hidden], [aria-hidden="true"]')) return false;
  const style = getElementComputedStyle(element);
  return !!style && style.display !== 'none' && style.visibility !== 'hidden' && hasBox(element, style);
}

function hasBox(element: Element, style: CSSStyleDeclaration): boolean {
  let rect: DOMRect;
  if (style.display === 'contents') {
    // No box of its own: its rendered children decide.
    const range = element.ownerDocument.createRange();
    range.selectNodeContents(element);
    rect = range.getBoundingClientRect();
  } else {
    rect = element.getBoundingClientRect();
  }
  return rect.width > 0 && rect.height > 0;
}

/** Text a field holds, the chosen option, a range's number. Never a password, never a link's. */
function valueOf(element: Element, tag: string, role: string): string | number | undefined {
  if (role === 'link') return undefined;
  if (tag === 'INPUT') {
    const input = element as HTMLInputElement;
    if (VALUELESS_INPUTS.has(input.type)) return undefined;
    return input.type === 'range' ? numeric(input.value) : valueText(input.value);
  }
  if (tag === 'TEXTAREA') return valueText((element as HTMLTextAreaElement).value);
  if (tag === 'SELECT') {
    return role === 'combobox' ? valueText((element as HTMLSelectElement).selectedOptions[0]?.text) : undefined;
  }
  if (tag === 'PROGRESS') {
    const progress = element as HTMLProgressElement;
    return progress.position < 0 ? undefined : numeric(String(progress.value));
  }
  if (tag === 'METER') return numeric(String((element as HTMLMeterElement).value));
  if (RANGE_ROLES.has(role)) {
    return valueText(element.getAttribute('aria-valuetext')) ?? numeric(element.getAttribute('aria-valuenow'));
  }
  if (isEditingHost(element)) return valueText((element as HTMLElement).innerText ?? element.textContent);
  return undefined;
}

function isEditingHost(element: Element): boolean {
  const editable = element.getAttribute('contenteditable')?.toLowerCase();
  return editable !== undefined && editable !== 'false';
}

function valueText(text: string | null | undefined): string | undefined {
  if (!text) return undefined;
  // Room for whitespace to collapse without flattening a megabyte of it.
  const value = cap(flat(text.length > MAX_VALUE * 8 ? text.slice(0, MAX_VALUE * 8) : text), MAX_VALUE);
  return value || undefined;
}

function numeric(text: string | null): number | undefined {
  if (text === null || !text.trim()) return undefined;
  const value = Number(text);
  return Number.isFinite(value) ? value : undefined;
}

function statesOf(element: Element, tag: string, role: string, focused: Element | null): WireStates | undefined {
  const states: WireStates = {};
  if (LEVEL_ROLES.has(role)) {
    const level = getAriaLevel(element);
    if (level) states.level = level;
  }
  if (CHECKED_ROLES.has(role) || ((role === 'option' || role === 'treeitem') && element.hasAttribute('aria-checked'))) {
    states.checked = String(getAriaChecked(element)) as WireStates['checked'];
  }
  // The <summary> carries the state of its <details>; a collapsed <select> is a closed combobox.
  const expanded =
    tag === 'DETAILS'
      ? undefined
      : (getAriaExpanded(element) ?? (tag === 'SELECT' && role === 'combobox' ? false : undefined));
  if (expanded !== undefined) states.expanded = expanded;
  if (getAriaSelected(element)) states.selected = true;
  if (getAriaDisabled(element)) states.disabled = true;
  if (isRequired(element, tag)) states.required = true;
  if (element === focused) states.focused = true;
  return Object.keys(states).length ? states : undefined;
}

function isRequired(element: Element, tag: string): boolean {
  if ((tag === 'INPUT' || tag === 'SELECT' || tag === 'TEXTAREA') && (element as HTMLInputElement).required)
    return true;
  return element.getAttribute('aria-required')?.toLowerCase() === 'true';
}

function hrefOf(element: Element): string | undefined {
  const href = (element as HTMLAnchorElement).href;
  return typeof href === 'string' && href && href.length <= MAX_URL ? href.toWellFormed() : undefined;
}

// ---------------------------------------------------------------------------------------------
// Hints: what tells apart controls the outline would otherwise print the same.

/**
 * The role and name the Bridge renders and keeps in a ref: a generic label around a hidden radio
 * or checkbox becomes that control, named by its text when it has no name.
 */
function asRendered(node: WireNode): { role: string; name: string; label: string } {
  let role = node.r;
  let name = node.n ?? '';
  const hidden = node.k?.hiddenInputType;
  if (role === 'generic' && (hidden === 'radio' || hidden === 'checkbox')) {
    role = hidden;
    name ||= node.k?.text ?? '';
  }
  return { role, name, label: name || node.k?.text || '' };
}

/**
 * A hint for every control whose role and label are empty or shared — the ones the Bridge shows a
 * hint for, and a few more: it counts across frames, this document only within itself.
 */
function addHints({ nodes, refable }: Walked): void {
  const rendered = refable.map(({ index }) => asRendered(nodes[index]));
  const counts = new Map<string, number>();
  for (const { role, label } of rendered) counts.set(`${role}\n${label}`, (counts.get(`${role}\n${label}`) ?? 0) + 1);
  refable.forEach(({ index, element }, at) => {
    const { role, label } = rendered[at];
    const node = nodes[index];
    const control = !!node.k || (INTERACTIVE_ROLES.has(role) && role !== 'Iframe');
    if (!control || (label && counts.get(`${role}\n${label}`) === 1)) return;
    const hint = hintFor(element);
    if (hint) node.h = hint;
  });
}

/** agent-browser's `HINT_FUNCTION`: the control's icon, image file, stable attribute and the row it sits in. */
function hintFor(element: Element): string {
  const tail = (text: string | null | undefined, separator: string) => (text ?? '').split(separator).pop() ?? '';
  const use = element.querySelector('svg use');
  const svg = element.querySelector('svg[id], svg[data-icon]');
  const mark = element.querySelector('svg[class], i[class]');
  const img = element.querySelector('img[src]:not([src^="data:"])');
  const icon = use
    ? tail(use.getAttribute('href') || use.getAttribute('xlink:href'), '#')
    : svg
      ? svg.id || svg.getAttribute('data-icon') || ''
      : mark
        ? (mark.getAttribute('class') ?? '')
            .split(/\s+/)
            .filter((name) => /^(lucide|fa|bi|mdi|ri|ti|icon)-/.test(name))
            .join(' ')
        : '';
  const file = img ? tail((img.getAttribute('src') ?? '').split(/[?#]/)[0], '/').replace(/\.\w+$/, '') : '';
  const key = ['data-testid', 'name', 'id'].find((name) =>
    /^(?!.*\d{3})[a-z][\w-]*$/i.test(element.getAttribute(name) || ''),
  );
  const tip = tooltipOf(element);
  const near = nearText(element);
  return [
    icon && `icon:${icon}`,
    file && `img:${file}`,
    key && `${key}:${element.getAttribute(key)}`,
    tip && `tip:${tip}`,
    near && `in:${near}`,
  ]
    .filter(Boolean)
    .map((part) => (part as string).slice(0, 48))
    .join('; ')
    .toWellFormed();
}

const renderedText = (element: Element): string => (element as HTMLElement).innerText ?? element.textContent ?? '';

const TOOLTIP = '[title], [data-tooltip]';
const ROW = 'tr, li, article, [role=row], [role=listitem], [role=option], [role=menuitem], [role=treeitem]';
/** Outside any row, this many ancestors are looked through for the words that go with a control. */
const NEAR_LEVELS = 3;
/** Words around a control longer than this belong to a container, not to the control's own group. */
const MAX_NEAR = 80;

/** What the control would say on hover: its own tooltip, else one inside it a hidden icon carries. */
function tooltipOf(element: Element): string {
  const holder = element.matches(TOOLTIP) ? element : element.querySelector(TOOLTIP);
  return flat(holder?.getAttribute('title') || holder?.getAttribute('data-tooltip') || '');
}

/**
 * The words beside a control in its row, or else in the nearest of its few ancestors that has any:
 * the text there, or another control's label (the menu arrow beside a labelled tool button).
 */
function nearText(element: Element): string {
  const row = element.closest(ROW);
  if (row) return textBesides(row, element);
  let at = element.parentElement;
  for (let level = 0; at && at !== document.body && level < NEAR_LEVELS; level++, at = at.parentElement) {
    const near = textBesides(at, element) || labelBesides(at, element);
    if (near) return near.length <= MAX_NEAR ? near : '';
  }
  return '';
}

function textBesides(container: Element, element: Element): string {
  return flat(renderedText(container).replace(renderedText(element), ''));
}

function labelBesides(container: Element, element: Element): string {
  for (const other of container.querySelectorAll('[aria-label]')) {
    if (element.contains(other) || other.contains(element)) continue;
    const label = flat(other.getAttribute('aria-label') ?? '');
    if (label) return label;
  }
  return '';
}

// ---------------------------------------------------------------------------------------------

const ZERO_WIDTH = /[\u200b-\u200d\u2060\ufeff\u00ad]/g;

/** One line of text as the Bridge compares and prints it: no zero-width characters, whitespace collapsed. */
function flat(text: string): string {
  return text.replace(ZERO_WIDTH, '').replace(/\s+/g, ' ').trim().toWellFormed();
}

/** At most `max` code points, so a cut never splits a character. */
function cap(text: string, max: number): string {
  if (text.length <= max) return text;
  let out = '';
  let count = 0;
  for (const char of text) {
    if (count++ === max) break;
    out += char;
  }
  return out;
}

function withAriaCaches<T>(run: () => T): T {
  beginAriaCaches();
  try {
    return run();
  } finally {
    endAriaCaches();
  }
}
