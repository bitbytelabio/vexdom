/**
 * Keys as `Input.dispatchKeyEvent` wants them, on a US layout: agent-browser's
 * `native/interaction.rs` (`char_to_key_info`, `named_key_info`, `modified_key_info`,
 * `shifted_char`, `key_location`, `key_modifier_bit`, `single_key_event_params`,
 * `press_key_with_chord_and_held`). Modifier bits are CDP's: 1 Alt, 2 Control, 4 Meta, 8 Shift.
 */

export type KeyInfo = [key: string, code: string, keyCode: number];

const PUNCTUATION: Record<string, [string, number]> = {
  ';': ['Semicolon', 186],
  ':': ['Semicolon', 186],
  '=': ['Equal', 187],
  '+': ['Equal', 187],
  ',': ['Comma', 188],
  '<': ['Comma', 188],
  '-': ['Minus', 189],
  _: ['Minus', 189],
  '.': ['Period', 190],
  '>': ['Period', 190],
  '/': ['Slash', 191],
  '?': ['Slash', 191],
  '`': ['Backquote', 192],
  '~': ['Backquote', 192],
  '[': ['BracketLeft', 219],
  '{': ['BracketLeft', 219],
  '\\': ['Backslash', 220],
  '|': ['Backslash', 220],
  ']': ['BracketRight', 221],
  '}': ['BracketRight', 221],
  "'": ['Quote', 222],
  '"': ['Quote', 222],
};

export function charToKeyInfo(ch: string): KeyInfo {
  if (ch === '\n' || ch === '\r') return ['Enter', 'Enter', 13];
  if (ch === '\t') return ['Tab', 'Tab', 9];
  if (ch === ' ') return [' ', 'Space', 32];
  if (/^[A-Za-z]$/.test(ch)) {
    const upper = ch.toUpperCase();
    return [ch, `Key${upper}`, upper.charCodeAt(0)];
  }
  if (/^[0-9]$/.test(ch)) return [ch, `Digit${ch}`, ch.charCodeAt(0)];
  const [code, keyCode] = PUNCTUATION[ch] ?? ['', 0];
  return [ch, code, keyCode];
}

const NAMED: Record<string, KeyInfo> = {
  enter: ['Enter', 'Enter', 13],
  return: ['Enter', 'Enter', 13],
  tab: ['Tab', 'Tab', 9],
  escape: ['Escape', 'Escape', 27],
  esc: ['Escape', 'Escape', 27],
  backspace: ['Backspace', 'Backspace', 8],
  delete: ['Delete', 'Delete', 46],
  arrowup: ['ArrowUp', 'ArrowUp', 38],
  up: ['ArrowUp', 'ArrowUp', 38],
  arrowdown: ['ArrowDown', 'ArrowDown', 40],
  down: ['ArrowDown', 'ArrowDown', 40],
  arrowleft: ['ArrowLeft', 'ArrowLeft', 37],
  left: ['ArrowLeft', 'ArrowLeft', 37],
  arrowright: ['ArrowRight', 'ArrowRight', 39],
  right: ['ArrowRight', 'ArrowRight', 39],
  home: ['Home', 'Home', 36],
  end: ['End', 'End', 35],
  pageup: ['PageUp', 'PageUp', 33],
  pagedown: ['PageDown', 'PageDown', 34],
  space: [' ', 'Space', 32],
  ' ': [' ', 'Space', 32],
  shift: ['Shift', 'ShiftLeft', 16],
  shiftleft: ['Shift', 'ShiftLeft', 16],
  shiftright: ['Shift', 'ShiftRight', 16],
  control: ['Control', 'ControlLeft', 17],
  ctrl: ['Control', 'ControlLeft', 17],
  controlleft: ['Control', 'ControlLeft', 17],
  controlright: ['Control', 'ControlRight', 17],
  alt: ['Alt', 'AltLeft', 18],
  altleft: ['Alt', 'AltLeft', 18],
  altright: ['Alt', 'AltRight', 18],
  meta: ['Meta', 'MetaLeft', 91],
  cmd: ['Meta', 'MetaLeft', 91],
  command: ['Meta', 'MetaLeft', 91],
  metaleft: ['Meta', 'MetaLeft', 91],
  metaright: ['Meta', 'MetaRight', 92],
};

export function namedKeyInfo(key: string): KeyInfo {
  const lower = key.toLowerCase();
  const named = NAMED[lower];
  if (named) return [...named];
  const coded = /^key([a-z])$/.exec(lower) ?? /^digit([0-9])$/.exec(lower);
  if (coded) return charToKeyInfo(coded[1]);
  if ([...key].length === 1) return charToKeyInfo(key);
  return [key, key, 0];
}

const SHIFTED: Record<string, string> = {
  '1': '!',
  '2': '@',
  '3': '#',
  '4': '$',
  '5': '%',
  '6': '^',
  '7': '&',
  '8': '*',
  '9': '(',
  '0': ')',
  '`': '~',
  '-': '_',
  '=': '+',
  '[': '{',
  ']': '}',
  '\\': '|',
  ';': ':',
  "'": '"',
  ',': '<',
  '.': '>',
  '/': '?',
};

export function shiftedChar(keyName: string): string | undefined {
  if ([...keyName].length !== 1) return undefined;
  if (/^[a-z]$/.test(keyName)) return keyName.toUpperCase();
  return SHIFTED[keyName];
}

export function keyText(keyName: string): string | undefined {
  if (keyName === 'Enter') return '\r';
  if (keyName === 'Tab') return '\t';
  if (keyName === ' ') return ' ';
  return keyName.length === 1 ? keyName : undefined;
}

export function keyModifierBit(key: string): number | undefined {
  switch (key.toLowerCase()) {
    case 'alt':
    case 'altleft':
    case 'altright':
      return 1;
    case 'control':
    case 'ctrl':
    case 'controlleft':
    case 'controlright':
      return 2;
    case 'meta':
    case 'cmd':
    case 'command':
    case 'metaleft':
    case 'metaright':
      return 4;
    case 'shift':
    case 'shiftleft':
    case 'shiftright':
      return 8;
    default:
      return undefined;
  }
}

function keyLocation(code: string): number | undefined {
  if (['ShiftLeft', 'ControlLeft', 'AltLeft', 'MetaLeft'].includes(code)) return 1;
  if (['ShiftRight', 'ControlRight', 'AltRight', 'MetaRight'].includes(code)) return 2;
  return undefined;
}

const nonzero = (modifiers: number) => (modifiers !== 0 ? modifiers : undefined);

/** `Input.dispatchKeyEvent` params for one key event. */
export function singleKeyEventParams(key: string, type: 'keyDown' | 'keyUp', modifiers?: number): Record<string, unknown> {
  const [unmodified, code, keyCode] = namedKeyInfo(key);
  const mask = modifiers ?? 0;
  const keyName = mask & 8 ? (shiftedChar(unmodified) ?? unmodified) : unmodified;
  const typed = mask & (1 | 2 | 4) ? undefined : keyText(keyName);
  const unmodifiedText = mask & (1 | 2 | 4) ? undefined : keyText(unmodified);
  const down = type === 'keyDown';
  const own = keyModifierBit(keyName);
  const params: Record<string, unknown> = {
    type,
    key: keyName,
    code,
    windowsVirtualKeyCode: keyCode,
  };
  const location = keyLocation(code);
  if (location !== undefined) params.location = location;
  if (down && typed !== undefined) params.text = typed;
  if (down && unmodifiedText !== undefined) params.unmodifiedText = unmodifiedText;
  const sent = down && own !== undefined ? (modifiers ?? 0) | own : modifiers;
  if (sent !== undefined) params.modifiers = sent;
  return params;
}

export type Dispatch = (params: Record<string, unknown>) => Promise<unknown>;

/** The chord's modifiers not already held go down first and come up last, around the key. */
export async function pressKeyWithChord(dispatch: Dispatch, key: string, chord: number, held: number): Promise<void> {
  const chordOnly = chord & ~held;
  const modifierKeys = (
    [
      [2, 'Control'],
      [1, 'Alt'],
      [8, 'Shift'],
      [4, 'Meta'],
    ] as const
  )
    .filter(([bit]) => chordOnly & bit)
    .map(([, name]) => name);
  let active = held;
  const pressed: string[] = [];
  try {
    for (const modifier of modifierKeys) {
      active |= keyModifierBit(modifier)!;
      await dispatch(singleKeyEventParams(modifier, 'keyDown', nonzero(active)));
      pressed.push(modifier);
    }
    const own = keyModifierBit(key) ?? 0;
    await dispatch(singleKeyEventParams(key, 'keyDown', nonzero(active | own)));
    await dispatch(singleKeyEventParams(key, 'keyUp', nonzero(active & ~(own & ~held))));
  } finally {
    for (const modifier of pressed.reverse()) {
      active &= ~keyModifierBit(modifier)!;
      await dispatch(singleKeyEventParams(modifier, 'keyUp', nonzero(active))).catch(() => undefined);
    }
  }
}

export const MODIFIER_BITS: Record<string, number> = { alt: 1, ctrl: 2, meta: 4, shift: 8 };
