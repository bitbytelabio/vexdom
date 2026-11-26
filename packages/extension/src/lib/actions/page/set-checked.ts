import { z } from 'zod';
import { ActionError, defineAction } from '../core';
import { assertActionable, dispatchClick, forceSchema } from './click-element';
import { computedRole, describeElement, isChecked, isVisible, resolveTarget, targetSchema } from './dom';

const CHECKABLE_ROLES = new Set(['checkbox', 'switch', 'radio']);

/** A label stands for its control: checking “Remember me” checks the box it labels. */
function controlOf(el: HTMLElement): HTMLElement {
  if (el instanceof HTMLLabelElement && el.control instanceof HTMLElement) return el.control;
  return el;
}

function isCheckable(el: HTMLElement): boolean {
  if (el instanceof HTMLInputElement) return el.type === 'checkbox' || el.type === 'radio';
  return CHECKABLE_ROLES.has(computedRole(el) ?? '');
}

const isRadio = (el: HTMLElement) =>
  (el instanceof HTMLInputElement && el.type === 'radio') || computedRole(el) === 'radio';

export const setChecked = defineAction({
  name: 'page.setChecked',
  description:
    'Check or uncheck a checkbox, switch or radio (or the control a label names). Does nothing when it is already in that state; a radio is cleared by choosing another option in its group.',
  input: z.object({
    target: targetSchema.describe('The checkbox, switch, radio, or its label'),
    checked: z.boolean().default(true).describe('false to uncheck'),
    force: forceSchema,
  }),
  execute({ target, checked, force }) {
    const el = controlOf(resolveTarget(target, { includeHidden: true }));
    if (!isCheckable(el)) {
      throw new ActionError(`<${el.tagName.toLowerCase()}> is not a checkbox, switch or radio`, 'NOT_CHECKABLE');
    }
    if (isChecked(el) === checked) return { changed: false, checked, element: describeElement(el) };
    if (!checked && isRadio(el)) {
      throw new ActionError('A radio is cleared by choosing another option in its group.', 'NOT_CHECKABLE');
    }
    el.scrollIntoView({ block: 'center', behavior: 'instant' });
    // A styled control often hides its native input behind its label: that one is only refused when disabled.
    if (!force) assertActionable(el, { visibility: isVisible(el) });
    dispatchClick(el);
    return { changed: true, checked: isChecked(el), element: describeElement(el) };
  },
});
