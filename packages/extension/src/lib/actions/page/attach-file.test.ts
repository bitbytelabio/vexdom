import { beforeEach, describe, expect, it } from 'vitest';
import { attachFile } from './attach-file';

// Happy DOM may lack the drag-and-drop classes; these are the few members the action and the page use.
if (typeof globalThis.DataTransfer === 'undefined') {
  class FakeDataTransfer {
    files: File[] = [];
    items = { add: (file: File) => this.files.push(file) };
  }
  Object.assign(globalThis, { DataTransfer: FakeDataTransfer });
}
if (
  typeof globalThis.DragEvent === 'undefined' ||
  !new DragEvent('drop', { dataTransfer: new DataTransfer() }).dataTransfer
) {
  class FakeDragEvent extends Event {
    dataTransfer: DataTransfer | null;
    constructor(type: string, init: EventInit & { dataTransfer?: DataTransfer | null } = {}) {
      super(type, init);
      this.dataTransfer = init.dataTransfer ?? null;
    }
  }
  Object.assign(globalThis, { DragEvent: FakeDragEvent });
}

const attach = (selector: string) =>
  attachFile.execute(
    attachFile.input.parse({ target: { selector }, name: 'a.png', mime: 'image/png', content: btoa('x') }),
  );

beforeEach(() => {
  document.body.innerHTML = '';
});

describe('attaching to a drop zone', () => {
  it('drops the files onto an element that is not a file input', () => {
    document.body.innerHTML = '<div id="zone">Drop files here</div>';
    let dropped: string | undefined;
    document.getElementById('zone')!.addEventListener('drop', (event) => {
      dropped = (event as DragEvent).dataTransfer?.files[0]?.name;
    });

    expect(attach('#zone')).toMatchObject({ attached: true, via: 'drop', name: 'a.png' });
    expect(dropped).toBe('a.png');
  });

  it('still refuses a text field', () => {
    document.body.innerHTML = '<input type="text" id="field">';
    expect(() => attach('#field')).toThrow(expect.objectContaining({ code: 'INVALID_TARGET' }));
  });
});
