import { useEffect, useState } from 'react';
import { listShortcuts, onShortcutsChange, type Shortcut } from './shortcuts';

/** The user's saved prompts, kept current as they change anywhere. */
export function useShortcuts(): Shortcut[] {
  const [shortcuts, setShortcuts] = useState<Shortcut[]>([]);
  useEffect(() => {
    let live = true;
    void listShortcuts().then((all) => {
      if (live) setShortcuts(all);
    });
    const stop = onShortcutsChange(setShortcuts);
    return () => {
      live = false;
      stop();
    };
  }, []);
  return shortcuts;
}
