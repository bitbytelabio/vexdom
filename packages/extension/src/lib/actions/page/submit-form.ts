import { z } from 'zod';
import { ActionError, defineAction } from '../core';
import { cssPath, resolveTarget, targetSchema } from './dom';

/** `ValidityState` flags, in the order a reason is picked. */
const REASONS = [
  'valueMissing',
  'typeMismatch',
  'patternMismatch',
  'tooLong',
  'tooShort',
  'rangeUnderflow',
  'rangeOverflow',
  'stepMismatch',
  'badInput',
  'customError',
] as const;

/**
 * The first fields that failed validation, each with why as a `ValidityState` flag. Not the
 * browser's message: for an email field it quotes the value, which may be a released secret.
 */
function invalidFields(form: HTMLFormElement): { field: string; reason: string }[] {
  return [...form.elements]
    .filter(
      (el): el is HTMLInputElement => (el as HTMLInputElement).willValidate && !(el as HTMLInputElement).validity.valid,
    )
    .slice(0, 5)
    .map((el) => ({ field: cssPath(el), reason: REASONS.find((flag) => el.validity[flag]) ?? 'invalid' }));
}

export const submitForm = defineAction({
  name: 'page.submitForm',
  description: 'Submit a form, firing its submit event and validation as if the user pressed Enter.',
  input: z.object({
    target: targetSchema
      .optional()
      .describe('The form, or any element inside it; defaults to the first form on the page'),
  }),
  execute({ target }) {
    const form = target ? resolveTarget(target).closest('form') : document.querySelector('form');
    if (!form) {
      throw new ActionError('No form found to submit', 'TARGET_NOT_FOUND');
    }
    // requestSubmit stops at the browser's own validation without a word: say so, and where.
    // Read without checkValidity(), which would fire every `invalid` event a second time.
    const invalid = form.noValidate ? [] : invalidFields(form);
    const valid = invalid.length === 0;
    if (typeof form.requestSubmit === 'function') {
      form.requestSubmit();
    } else {
      form.submit();
    }
    return {
      form: cssPath(form),
      action: form.getAttribute('action') || undefined,
      method: (form.getAttribute('method') || 'get').toLowerCase(),
      submitted: valid,
      ...(valid ? {} : { invalid }),
    };
  },
});
