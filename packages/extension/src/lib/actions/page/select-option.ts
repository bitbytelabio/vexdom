import { z } from 'zod';
import { ActionError, defineAction } from '../core';
import { dispatchClick } from './click-element';
import { accessibleText, isVisible, resolveTarget, targetSchema } from './dom';

const OPTIONS_WAIT_MS = 1_000;
const LISTED_OPTIONS = 20;

export const selectOption = defineAction({
  name: 'page.selectOption',
  description:
    'Choose an option in a dropdown by value, visible label, or position: a native <select> (several values for a <select multiple>), or a custom ARIA listbox or combobox, which is opened and its [role=option] clicked.',
  input: z.object({
    target: targetSchema.describe('The select element, or the button that opens a custom dropdown'),
    value: z.string().optional().describe('Match by option value'),
    label: z.string().optional().describe('Match by visible option text, case-insensitive'),
    index: z.number().int().nonnegative().optional().describe('Match by option position'),
    values: z
      .array(z.string())
      .optional()
      .describe('For a <select multiple>: every option to select, by value or label; the rest are cleared'),
  }),
  async execute({ target, value, label, index, values }) {
    const el = resolveTarget(target);
    if (!(el instanceof HTMLSelectElement)) return chooseAria(el, { value, label, index });
    const options = [...el.options];
    const labelOf = (opt: HTMLOptionElement) => (opt.label || opt.text).trim();

    if (values) {
      if (!el.multiple) throw new ActionError('This <select> takes one option; give "value" or "label".', 'NOT_MULTIPLE');
      const matches = (opt: HTMLOptionElement, wanted: string) =>
        opt.value === wanted || labelOf(opt).toLowerCase() === wanted.trim().toLowerCase();
      const missing = values.filter((wanted) => !options.some((opt) => matches(opt, wanted)));
      if (missing.length) throw optionNotFound(missing.join('", "'), options.map(labelOf));
      for (const opt of options) opt.selected = values.some((wanted) => matches(opt, wanted));
      el.dispatchEvent(new Event('input', { bubbles: true }));
      el.dispatchEvent(new Event('change', { bubbles: true }));
      return { selected: options.filter((opt) => opt.selected).map((opt) => ({ value: opt.value, label: labelOf(opt), index: opt.index })) };
    }

    let option: HTMLOptionElement | undefined;
    if (value !== undefined) {
      option = options.find((opt) => opt.value === value);
    } else if (label !== undefined) {
      const needle = label.trim().toLowerCase();
      option = options.find((opt) => labelOf(opt).toLowerCase() === needle);
    } else if (index !== undefined) {
      option = options[index];
    } else {
      throw new ActionError('Provide a "value", "label", or "index" to choose', 'INVALID_INPUT');
    }
    if (!option) {
      const labels = options.slice(0, LISTED_OPTIONS).map(labelOf);
      throw new ActionError(
        `No option matches ${JSON.stringify({ value, label, index })}; options are: ${labels.join(', ')}`,
        'TARGET_NOT_FOUND',
      );
    }
    Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, 'value')!.set!.call(
      el,
      option.value,
    );
    el.dispatchEvent(new Event('input', { bubbles: true }));
    el.dispatchEvent(new Event('change', { bubbles: true }));
    return {
      selected: { value: option.value, label: option.label || option.text, index: option.index },
    };
  },
});

function optionNotFound(wanted: string, names: string[]): ActionError {
  return new ActionError(
    `No option matches "${wanted}". Options: ${names.slice(0, LISTED_OPTIONS).join(', ')}`,
    'OPTION_NOT_FOUND',
  );
}

const nameOf = (option: Element) => accessibleText(option).replace(/\s+/g, ' ').trim();
const valueOf = (option: Element) => option.getAttribute('data-value') ?? option.getAttribute('value');

/** The options a custom dropdown shows: inside what it controls or owns, else anywhere visible. */
function ariaOptions(el: Element): HTMLElement[] {
  const ids = `${el.getAttribute('aria-controls') ?? ''} ${el.getAttribute('aria-owns') ?? ''}`.split(/\s+/).filter(Boolean);
  const owned = ids.flatMap((id) => [...(document.getElementById(id)?.querySelectorAll<HTMLElement>('[role=option]') ?? [])]);
  if (owned.length) return owned;
  return [...document.querySelectorAll<HTMLElement>('[role=option]')].filter(isVisible);
}

function nextFrame(): Promise<void> {
  return new Promise((resolve) => requestAnimationFrame(() => resolve()));
}

async function chooseAria(el: HTMLElement, wanted: { value?: string; label?: string; index?: number }) {
  if (wanted.value === undefined && wanted.label === undefined && wanted.index === undefined) {
    throw new ActionError('Provide a "value", "label", or "index" to choose', 'INVALID_INPUT');
  }
  dispatchClick(el);
  const deadline = Date.now() + OPTIONS_WAIT_MS;
  let options = ariaOptions(el);
  while (!options.length && Date.now() < deadline) {
    await nextFrame();
    options = ariaOptions(el);
  }

  let option: HTMLElement | undefined;
  const shown = String(wanted.value ?? wanted.label ?? wanted.index);
  if (wanted.index !== undefined && wanted.value === undefined && wanted.label === undefined) {
    option = options[wanted.index];
  } else {
    const text = (wanted.value ?? wanted.label)!;
    const byValue = options.find((opt) => valueOf(opt) === text);
    const byName = options.find((opt) => nameOf(opt).toLowerCase() === text.trim().toLowerCase());
    if (byValue && byName && byValue !== byName) {
      throw new ActionError(
        `"${text}" is one option's value and another option's label — say which with "value" or "label" on a native select, or use "index".`,
        'AMBIGUOUS',
      );
    }
    option = wanted.value !== undefined ? (byValue ?? byName) : (byName ?? byValue);
  }
  if (!option) throw optionNotFound(shown, options.map(nameOf));
  if (option.getAttribute('aria-disabled') === 'true') {
    throw new ActionError(`The option "${nameOf(option)}" is disabled.`, 'OPTION_DISABLED');
  }
  option.scrollIntoView({ block: 'nearest', behavior: 'instant' });
  dispatchClick(option);
  return { selected: { value: valueOf(option) ?? nameOf(option), label: nameOf(option), index: options.indexOf(option) } };
}
