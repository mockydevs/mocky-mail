import '@testing-library/jest-dom/vitest';
import { cleanup } from '@testing-library/react';
import { afterEach } from 'vitest';

// jsdom does no layout, and the rich-text editor (ProseMirror) asks where
// the cursor is to scroll it into view. Empty rectangles are enough.
const emptyRect = (): DOMRect => new DOMRect(0, 0, 0, 0);
const emptyRects = () => Object.assign([], { item: () => null });
for (const proto of [Range.prototype, Element.prototype]) {
  if (!('getClientRects' in proto) || typeof proto.getClientRects !== 'function') {
    Object.defineProperty(proto, 'getClientRects', { configurable: true, value: emptyRects });
  }
  if (!('getBoundingClientRect' in proto) || typeof proto.getBoundingClientRect !== 'function') {
    Object.defineProperty(proto, 'getBoundingClientRect', { configurable: true, value: emptyRect });
  }
}
// jsdom doesn't compute isContentEditable either; the app's shortcuts use it
// to leave keys typed in the editor alone.
if (!('isContentEditable' in HTMLElement.prototype)) {
  Object.defineProperty(HTMLElement.prototype, 'isContentEditable', {
    configurable: true,
    get(this: HTMLElement) {
      const set = this.closest('[contenteditable]')?.getAttribute('contenteditable');
      return set === '' || set === 'true';
    },
  });
}
if (typeof document.elementFromPoint !== 'function') {
  Object.defineProperty(document, 'elementFromPoint', { configurable: true, value: () => null });
}

afterEach(() => {
  cleanup();
  window.localStorage.clear();
  delete document.documentElement.dataset['theme'];
});
