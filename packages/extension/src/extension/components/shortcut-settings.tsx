import { useState } from 'react';

import { Row, SettingsGroup } from '@/extension/components/extension-settings';
import { Button } from '@/extension/components/ui/button';
import { Input } from '@/extension/components/ui/input';
import { Textarea } from '@/extension/components/ui/textarea';
import { removeShortcut, saveShortcut, type Shortcut } from '@/lib/bridge/shortcuts';
import { useShortcuts } from '@/lib/bridge/use-shortcuts';
import { MAX_INSTRUCTION_CHARS } from '@/lib/schedules/task';

const NOTE_CHARS = 120;

export function ShortcutSettings() {
  const shortcuts = useShortcuts();
  const [editing, setEditing] = useState<string | null>(null);
  return (
    <>
      <SettingsGroup
        title="New shortcut"
        note="A name you type after / in the composer, and the prompt it puts there for you to edit and send."
      >
        <div className="p-4">
          <ShortcutForm onDone={() => undefined} />
        </div>
      </SettingsGroup>
      <SettingsGroup title="Your shortcuts" note={shortcuts.length ? undefined : 'None yet.'}>
        {shortcuts.map((shortcut) =>
          editing === shortcut.id ? (
            <div key={shortcut.id} className="p-4">
              <ShortcutForm shortcut={shortcut} onDone={() => setEditing(null)} />
            </div>
          ) : (
            <Row key={shortcut.id} title={`/${shortcut.name}`} note={clip(shortcut.prompt)}>
              <div className="flex shrink-0 items-center gap-1.5">
                <Button variant="ghost" size="sm" onClick={() => setEditing(shortcut.id)}>
                  Edit
                </Button>
                <Button variant="ghost" size="sm" onClick={() => void removeShortcut(shortcut.id)}>
                  Delete
                </Button>
              </div>
            </Row>
          ),
        )}
      </SettingsGroup>
    </>
  );
}

function ShortcutForm({ shortcut, onDone }: { shortcut?: Shortcut; onDone: () => void }) {
  const [name, setName] = useState(shortcut?.name ?? '');
  const [prompt, setPrompt] = useState(shortcut?.prompt ?? '');
  const [error, setError] = useState<string | null>(null);

  async function save() {
    const saved = await saveShortcut({ id: shortcut?.id, name, prompt });
    if (!saved.ok) {
      setError(saved.error);
      return;
    }
    setError(null);
    if (!shortcut) {
      setName('');
      setPrompt('');
    }
    onDone();
  }

  return (
    <form
      className="space-y-2.5"
      onSubmit={(event) => {
        event.preventDefault();
        void save();
      }}
    >
      <label className="flex items-center gap-1.5">
        <span className="font-mono text-sm text-ink-faint">/</span>
        <Input
          value={name}
          onChange={(event) => setName(event.target.value)}
          placeholder="name"
          aria-label="Shortcut name"
        />
      </label>
      <Textarea
        value={prompt}
        onChange={(event) => setPrompt(event.target.value)}
        placeholder="The prompt it puts in the composer"
        aria-label="Shortcut prompt"
        maxLength={MAX_INSTRUCTION_CHARS}
        rows={3}
      />
      {error && <p className="text-xs text-amber">{error}</p>}
      <div className="flex justify-end gap-1.5">
        {shortcut && (
          <Button type="button" variant="ghost" size="sm" onClick={onDone}>
            Cancel
          </Button>
        )}
        <Button type="submit" size="sm">
          {shortcut ? 'Save' : 'Add shortcut'}
        </Button>
      </div>
    </form>
  );
}

const clip = (text: string) => {
  const oneLine = text.replace(/\s+/g, ' ');
  return oneLine.length > NOTE_CHARS ? `${oneLine.slice(0, NOTE_CHARS - 1)}…` : oneLine;
};
