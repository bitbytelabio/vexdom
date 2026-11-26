import { beforeEach, describe, expect, it } from 'vitest';
import { typeInto } from './fill-input';

const typed = (html: string, value: string) => {
  document.body.innerHTML = html;
  const el = document.body.firstElementChild as HTMLInputElement;
  typeInto(el, value, { clear: true, pressEnter: false });
  return el.value;
};

const refusal = (html: string, value: string): string => {
  try {
    typed(html, value);
  } catch (error) {
    return `${(error as { code?: string }).code}: ${(error as Error).message}`;
  }
  return 'accepted';
};

beforeEach(() => {
  document.body.innerHTML = '';
});

describe('typing into inputs with a fixed format', () => {
  it.each([
    ['date', '2026-11-26', 'tomorrow'],
    ['time', '13:45', '1pm'],
    ['datetime-local', '2026-11-26T13:45', '2026-11-26 13:45'],
    ['month', '2026-11', 'November'],
    ['week', '2026-W48', '2026-48'],
    ['color', '#ff8800', 'orange'],
    ['number', '42', 'forty'],
  ])('%s takes %s and refuses %s', (type, good, bad) => {
    expect(typed(`<input type="${type}">`, good)).toBe(good);
    expect(refusal(`<input type="${type}">`, bad)).toMatch(new RegExp(`^INVALID_INPUT: ${type} inputs take `));
  });

  it('keeps a range inside its bounds', () => {
    expect(typed('<input type="range" min="0" max="10">', '7')).toBe('7');
    expect(refusal('<input type="range" min="0" max="10">', '11')).toBe(
      'INVALID_INPUT: range inputs take a number from 0 to 10, e.g. 0',
    );
  });
});

describe('fields that cannot take text', () => {
  it('refuses read-only, disabled, checkable and select fields', () => {
    expect(refusal('<input readonly>', 'x')).toMatch(/^INVALID_TARGET: .* is read-only$/);
    expect(refusal('<input disabled>', 'x')).toMatch(/^INVALID_TARGET: .* is disabled$/);
    expect(refusal('<input type="checkbox">', 'x')).toMatch(/is a checkbox; use check or uncheck$/);
    expect(refusal('<select><option>a</option></select>', 'a')).toMatch(/is a select; use selectOption$/);
  });

  it('still fills a plain text field', () => {
    expect(typed('<input>', 'Ada')).toBe('Ada');
  });
});
