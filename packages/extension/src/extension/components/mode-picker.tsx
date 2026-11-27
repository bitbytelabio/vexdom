import { useState } from 'react';
import { ListChecks, Shield, ShieldOff, ShieldQuestion, type LucideIcon } from 'lucide-react';

import { Popover } from '@/extension/components/popover';
import { Button } from '@/extension/components/ui/button';
import { usePreferences } from '@/lib/bridge/use-preferences';
import { MODE_LABEL, MODE_SETTING, PERMISSION_MODES, type PermissionMode } from '@/lib/settings/guardrails';
import { cn } from '@/lib/utils';

const ICONS: Record<PermissionMode, LucideIcon> = {
  ask: ShieldQuestion,
  plan: ListChecks,
  rules: Shield,
  auto: ShieldOff,
};

/** The composer's permission mode: how the next run asks before it acts. */
export function ModePicker() {
  const { connected, preferences, change } = usePreferences();
  const [open, setOpen] = useState(false);
  const mode = preferences?.guardrails.mode ?? 'rules';
  const Icon = ICONS[mode];

  function choose(next: PermissionMode) {
    setOpen(false);
    if (next !== mode) void change({ kind: 'guardrail', setting: MODE_SETTING, value: next }, MODE_SETTING);
  }

  return (
    <div className="relative">
      <Button
        variant="ghost"
        size="icon-sm"
        aria-label={`Permission mode: ${MODE_LABEL[mode].title}`}
        aria-expanded={open}
        title={`${MODE_LABEL[mode].title} — ${MODE_LABEL[mode].note}`}
        onClick={() => setOpen((shown) => !shown)}
        disabled={!connected || !preferences}
        className={cn(mode === 'auto' && 'text-ember')}
      >
        <Icon className="size-3.5" />
      </Button>
      <Popover
        open={open}
        label="Permission mode"
        side="top"
        align="start"
        onClose={() => setOpen(false)}
        className="w-72"
      >
        <div role="radiogroup" aria-label="Permission mode" className="flex flex-col gap-0.5 p-1.5">
          {PERMISSION_MODES.map((option) => {
            const OptionIcon = ICONS[option];
            return (
              <button
                key={option}
                type="button"
                role="radio"
                aria-checked={option === mode}
                onClick={() => choose(option)}
                className={cn(
                  'flex items-start gap-2 rounded-lg px-2.5 py-2 text-left transition-colors',
                  option === mode ? 'bg-brand/12' : 'hover:bg-ground/40',
                )}
              >
                <OptionIcon
                  className={cn('mt-0.5 size-3.5 shrink-0', option === 'auto' ? 'text-ember' : 'text-ink-dim')}
                />
                <span className="min-w-0">
                  <span className="block text-xs font-medium text-ink">{MODE_LABEL[option].title}</span>
                  <span className="block text-[11px] leading-snug text-ink-dim">{MODE_LABEL[option].note}</span>
                </span>
              </button>
            );
          })}
          <p className="px-2.5 pt-1 pb-0.5 text-[10px] text-ink-faint">Applies from your next message.</p>
        </div>
      </Popover>
    </div>
  );
}
