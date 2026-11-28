import { afterEach, describe, expect, it } from 'vitest';
import { resolveAll, targetSchema } from './dom';

const target = (input: object) => targetSchema.parse(input);

describe('a text locator', () => {
  afterEach(() => {
    document.body.innerHTML = '';
  });

  it('takes the element whose whole text it is over one that merely contains it', () => {
    document.body.innerHTML = '<button>More technicals</button><button>Technicals</button>';
    const [only, ...rest] = resolveAll(target({ text: 'technicals' }), { includeHidden: true });
    expect(only.textContent).toBe('Technicals');
    expect(rest).toEqual([]);
  });

  it('still finds an element that contains the text when none is exactly it', () => {
    document.body.innerHTML = '<button>More technicals</button><button>Fewer technicals</button>';
    expect(resolveAll(target({ text: 'technicals' }), { includeHidden: true })).toHaveLength(2);
  });
});
