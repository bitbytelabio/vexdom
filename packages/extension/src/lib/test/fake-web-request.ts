// fake-browser leaves `webRequest` unimplemented (its listeners throw), and every action a test
// routes into a tab starts a request watch (src/lib/bridge/request-watch.ts). Events that keep
// their listeners and never fire stand in; a test that needs requests defines its own.
import { fakeBrowser } from 'wxt/testing';

const silentEvent = () => {
  const listeners = new Set<unknown>();
  return {
    addListener: (listener: unknown) => void listeners.add(listener),
    removeListener: (listener: unknown) => void listeners.delete(listener),
    hasListener: (listener: unknown) => listeners.has(listener),
  };
};

Object.defineProperty(fakeBrowser, 'webRequest', {
  value: { onBeforeRequest: silentEvent(), onCompleted: silentEvent(), onErrorOccurred: silentEvent() },
  configurable: true,
});
