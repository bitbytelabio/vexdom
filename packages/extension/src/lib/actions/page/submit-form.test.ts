import { beforeEach, describe, expect, it } from 'vitest';
import { submitForm } from './submit-form';

const submit = () => submitForm.execute(submitForm.input.parse({})) as Record<string, unknown>;

beforeEach(() => {
  document.body.innerHTML = '';
});

describe('submitting a form', () => {
  it('says the browser stopped it, and at which field, when a field is invalid', () => {
    document.body.innerHTML = '<form><input id="email" required><button>Send</button></form>';
    const result = submit();
    expect(result.submitted).toBe(false);
    expect(result.invalid).toEqual([{ field: expect.stringContaining('email'), reason: 'valueMissing' }]);
  });

  it('reports a valid form as submitted, without an invalid list', () => {
    document.body.innerHTML = '<form><input id="email" required value="a@b.c"></form>';
    const result = submit();
    expect(result.submitted).toBe(true);
    expect(result).not.toHaveProperty('invalid');
  });
});
