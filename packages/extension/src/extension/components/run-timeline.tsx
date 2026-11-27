import { useEffect, useState } from 'react';
import {
  Braces,
  CalendarClock,
  Camera,
  Check,
  Clapperboard,
  Code2,
  Compass,
  CornerDownLeft,
  ExternalLink,
  Eye,
  Highlighter,
  Hourglass,
  Info,
  Keyboard,
  ListChecks,
  MousePointerClick,
  MoveVertical,
  Paperclip,
  Radar,
  ScanEye,
  Terminal,
  TextCursorInput,
  TriangleAlert,
  Type,
  Wrench,
  X,
  Zap,
  type LucideIcon,
} from 'lucide-react';

import { CodeReview } from '@/extension/components/code-review';
import { ContextCard } from '@/extension/components/context-card';
import { Markdown } from '@/extension/components/markdown';
import { Button } from '@/extension/components/ui/button';
import { injectCode } from '@/lib/actions/page/inject-code';
import { HAND_OFF_ACTION, PROPOSE_PLAN_ACTION } from '@/lib/actions/reserved';
import { openScreenshot } from '@/lib/bridge/screenshot-preview';
import type { RunItem } from '@/lib/bridge/run-items';
import { cn } from '@/lib/utils';

interface RunTimelineProps {
  items: RunItem[];
  running: boolean;
  onDecide: (toolId: string, allow: boolean, remember?: boolean) => void;
  onSchedule?: (text: string) => void;
}

export function RunTimeline({ items, running, onDecide, onSchedule }: RunTimelineProps) {
  const last = items.at(-1);
  const streaming = running && last?.kind === 'assistant';
  const awaiting = running && !streaming;
  const turnStart = items.findLastIndex((item) => item.kind === 'user');

  return (
    <div className="flex min-w-0 flex-col gap-2.5 p-3">
      {items.map((item, index) =>
        item.kind === 'tool' ? (
          <ToolRow key={item.id} item={item} live={running && index > turnStart} onDecide={onDecide} />
        ) : item.kind === 'notice' ? (
          <Notice key={item.id} item={item} />
        ) : item.kind === 'context' ? (
          <ContextCard key={item.id} breakdown={item.breakdown} />
        ) : item.kind === 'user' ? (
          <UserBubble
            key={item.id}
            text={item.text}
            focus={item.focus}
            onSchedule={onSchedule && !running ? () => onSchedule(item.text) : undefined}
          />
        ) : (
          <Reply key={item.id} text={item.text} streaming={streaming && index === items.length - 1} />
        ),
      )}
      {awaiting && <Thinking />}
    </div>
  );
}

function UserBubble({ text, focus, onSchedule }: { text: string; focus?: string; onSchedule?: () => void }) {
  return (
    <div className="enters group flex min-w-0 flex-col items-end gap-1">
      <div className="min-w-0 max-w-[88%] rounded-2xl rounded-br-md border border-brand/30 bg-brand/10 px-3 py-2 text-sm whitespace-pre-wrap wrap-anywhere text-ink">
        {text}
      </div>
      {focus && (
        <span className="flex min-w-0 max-w-[88%] items-center gap-1 rounded-full bg-ember/12 px-2 py-0.5 text-[10px] text-ember">
          <ScanEye className="size-2.5 shrink-0" />
          <span className="truncate">{focus}</span>
        </span>
      )}
      {onSchedule && (
        <button
          type="button"
          onClick={onSchedule}
          className="flex items-center gap-1 rounded-full px-1.5 py-0.5 text-[10px] text-ink-faint opacity-0 transition-opacity group-hover:opacity-100 hover:text-brand focus-visible:opacity-100"
        >
          <CalendarClock className="size-2.5" /> Repeat this…
        </button>
      )}
    </div>
  );
}

function Reply({ text, streaming }: { text: string; streaming: boolean }) {
  return (
    <div className="enters min-w-0">
      <Markdown
        text={text}
        streaming={streaming}
        className="panel-card rounded-2xl rounded-tl-md px-3 py-2.5 text-sm leading-relaxed text-ink"
      />
    </div>
  );
}

function Thinking() {
  return (
    <div className="flex items-center gap-2 px-1">
      <span className="flex items-center gap-1">
        {[0, 1, 2].map((dot) => (
          <span key={dot} className="think-dot size-1.5 rounded-full bg-brand" />
        ))}
      </span>
      <span className="font-mono text-[10px] tracking-[0.14em] text-ink-faint uppercase">Working</span>
    </div>
  );
}

function ToolRow({
  item,
  live,
  onDecide,
}: {
  item: Extract<RunItem, { kind: 'tool' }>;
  /** In the turn still running. A row a CLI never closed stops sweeping when its turn ends. */
  live: boolean;
  onDecide: (toolId: string, allow: boolean, remember?: boolean) => void;
}) {
  const handOff = item.action === HAND_OFF_ACTION;
  const plan = item.action === PROPOSE_PLAN_ACTION ? planOf(item.input) : null;
  const name = handOff ? 'hand-off' : item.action === PROPOSE_PLAN_ACTION ? 'plan' : item.action.replace(/^page\./, '');
  const Icon = ICONS[name] ?? Wrench;
  const pending = live && item.ok === undefined && !item.awaiting;
  const source = item.action === injectCode.name ? sourceOf(item.input) : null;
  const [reviewing, setReviewing] = useState(false);

  useEffect(() => {
    if (!item.awaiting) setReviewing(false);
  }, [item.awaiting]);

  return (
    <div className="enters flex min-w-0 flex-col gap-1.5">
      <div
        className={cn(
          'relative flex min-w-0 items-start gap-2 overflow-hidden rounded-lg border px-2 py-1.5 transition-colors',
          item.awaiting
            ? 'border-ember/45 bg-ember/10'
            : item.ok === false
              ? 'border-destructive/40 bg-destructive/8'
              : 'border-line bg-ground/40',
          pending && 'sweep',
        )}
      >
        <Icon
          className={cn(
            'mt-0.5 size-3 shrink-0',
            item.awaiting
              ? 'text-ember'
              : item.ok === false
                ? 'text-destructive'
                : item.source === 'local'
                  ? 'text-amber'
                  : item.source === 'external'
                    ? 'text-magenta'
                    : 'text-brand',
          )}
        />
        <div className="min-w-0 flex-1">
          <div className="flex min-w-0 items-center gap-1.5">
            <span className="truncate font-mono text-[10px] text-ink">{name}</span>
            {item.source === 'local' && <Tag tone="amber" icon={Zap} label="local" />}
            {item.source === 'external' && <Tag tone="magenta" icon={Terminal} label="mcp" />}
          </div>
          {item.summary && (
            <p className={cn('mt-0.5 truncate text-[10px]', item.ok === false ? 'text-destructive' : 'text-ink-faint')}>
              {item.summary}
            </p>
          )}
        </div>
        {item.ok === true && <Check className="mt-0.5 size-3 shrink-0 text-lime/80" />}
        {item.ok === false && <X className="mt-0.5 size-3 shrink-0 text-destructive" />}
      </div>

      {item.preview && (
        <button
          type="button"
          onClick={() => void openScreenshot(item.preview!)}
          title={`Open the ${item.preview.full ? 'full-size' : 'captured'} screenshot in a new tab`}
          className="group max-w-[88%] overflow-hidden rounded-xl border border-line bg-ground/60 transition-colors hover:border-brand/50"
        >
          <img
            src={item.preview.thumbnail}
            alt={`Screenshot, ${item.preview.width}×${item.preview.height}`}
            width={item.preview.width}
            height={item.preview.height}
            className="block h-auto w-full transition-opacity group-hover:opacity-85"
          />
        </button>
      )}

      {item.awaiting && handOff && (
        <div className="flex flex-col gap-2 rounded-lg border border-amber/45 bg-amber/10 px-2.5 py-2">
          <p className="font-mono text-[10px] tracking-[0.14em] text-amber uppercase">Your turn</p>
          <p className="text-xs leading-relaxed text-ink">{reasonOf(item.input)}</p>
          <div className="flex flex-wrap justify-end gap-1.5">
            <Button size="sm" variant="ghost" onClick={() => onDecide(item.id, false)}>
              Stop
            </Button>
            <Button size="sm" onClick={() => onDecide(item.id, true)}>
              Done — continue
            </Button>
          </div>
        </div>
      )}

      {item.awaiting && plan && (
        <div className="flex flex-col gap-2 rounded-lg border border-ember/45 bg-ember/10 px-2.5 py-2">
          <p className="font-mono text-[10px] tracking-[0.14em] text-ember uppercase">Plan</p>
          <div className="flex flex-wrap items-center gap-1">
            <span className="text-[11px] text-ink-dim">Sites, with their subdomains</span>
            {plan.domains.map((domain) => (
              <span
                key={domain}
                className="rounded-full border border-line bg-ground/60 px-2 py-0.5 font-mono text-[10px] text-ink"
              >
                {domain}
              </span>
            ))}
          </div>
          <ol className="list-decimal space-y-0.5 pl-5 text-xs text-ink">
            {plan.steps.map((step, index) => (
              <li key={index}>{step}</li>
            ))}
          </ol>
          <div className="flex flex-wrap justify-end gap-1.5">
            <Button size="sm" variant="ghost" onClick={() => onDecide(item.id, false)}>
              Reject
            </Button>
            <Button size="sm" variant="ghost" onClick={() => onDecide(item.id, true, true)}>
              Approve and always allow these sites
            </Button>
            <Button size="sm" onClick={() => onDecide(item.id, true)}>
              Approve plan
            </Button>
          </div>
        </div>
      )}

      {item.awaiting && !handOff && !plan && (
        <div className="flex flex-wrap items-center gap-1.5 rounded-lg border border-ember/45 bg-ember/10 px-2.5 py-2">
          <p className="flex-1 text-xs text-ink">{source ? 'Allow this code to run?' : 'Allow this action?'}</p>
          {source && (
            <Button size="sm" variant="ghost" onClick={() => setReviewing(true)} title="Read the code before deciding">
              Review
            </Button>
          )}
          <Button size="sm" variant="ghost" onClick={() => onDecide(item.id, false)}>
            Deny
          </Button>
          {item.site && !source && (
            <Button
              size="sm"
              variant="ghost"
              onClick={() => onDecide(item.id, true, true)}
              title={`Don’t ask again for this on ${item.site}. Undo in Settings → Guardrails.`}
            >
              Always on {item.site}
            </Button>
          )}
          <Button size="sm" onClick={() => onDecide(item.id, true)}>
            Allow
          </Button>
        </div>
      )}

      {reviewing && source && (
        <CodeReview
          purpose={source.purpose}
          code={source.code}
          site={item.site}
          onAllow={() => onDecide(item.id, true)}
          onDeny={() => onDecide(item.id, false)}
          onClose={() => setReviewing(false)}
        />
      )}
    </div>
  );
}

function reasonOf(input: unknown): string {
  const reason = (input as { reason?: unknown } | null)?.reason;
  return typeof reason === 'string' && reason.trim() ? reason : 'The agent needs you to do something on the page.';
}

function planOf(input: unknown): { domains: string[]; steps: string[] } | null {
  const args = input as { domains?: unknown; steps?: unknown } | null;
  const strings = (value: unknown) =>
    Array.isArray(value) ? value.filter((entry): entry is string => typeof entry === 'string') : [];
  const domains = strings(args?.domains);
  const steps = strings(args?.steps);
  return domains.length || steps.length ? { domains, steps } : null;
}

function sourceOf(input: unknown): { purpose: string; code: string } | null {
  const args = input as { purpose?: unknown; code?: unknown } | null;
  if (typeof args?.code !== 'string' || !args.code) return null;
  return { purpose: typeof args.purpose === 'string' ? args.purpose : 'No purpose given.', code: args.code };
}

function Tag({ tone, icon: Icon, label }: { tone: 'amber' | 'magenta'; icon: LucideIcon; label: string }) {
  return (
    <span
      className={cn(
        'inline-flex shrink-0 items-center gap-0.5 rounded-full px-1.5 py-px font-mono text-[8px] tracking-[0.1em] uppercase',
        tone === 'amber' ? 'bg-amber/15 text-amber' : 'bg-magenta/15 text-magenta',
      )}
    >
      <Icon className="size-2" />
      {label}
    </span>
  );
}

function Notice({ item }: { item: Extract<RunItem, { kind: 'notice' }> }) {
  const isError = item.tone === 'error';
  return (
    <div
      className={cn(
        'enters flex items-start gap-2 rounded-lg px-2.5 py-1.5 text-[11px] leading-relaxed',
        isError ? 'border border-destructive/35 bg-destructive/8 text-destructive' : 'text-ink-faint',
      )}
    >
      {isError ? <TriangleAlert className="mt-px size-3 shrink-0" /> : <Info className="mt-px size-3 shrink-0" />}
      <span className="min-w-0 break-words">{item.text}</span>
    </div>
  );
}

const ICONS: Record<string, LucideIcon> = {
  getPageInfo: Eye,
  extractText: Eye,
  pickElement: ScanEye,
  'vexdom.focusShot': ScanEye,
  'vexdom.readFile': Paperclip,
  findProgress: Eye,
  clickElement: MousePointerClick,
  trustedClick: MousePointerClick,
  hoverElement: MousePointerClick,
  fillInput: Type,
  typeText: Type,
  focusInput: TextCursorInput,
  selectText: TextCursorInput,
  pressKey: Keyboard,
  submitForm: CornerDownLeft,
  selectOption: ListChecks,
  navigate: Compass,
  openTab: ExternalLink,
  switchTab: ExternalLink,
  closeTab: X,
  screenshot: Camera,
  scrollTo: MoveVertical,
  highlightElement: Highlighter,
  waitForElement: Hourglass,
  attachFile: Paperclip,
  listFiles: Paperclip,
  startMonitor: Radar,
  stopMonitor: Radar,
  monitorStatus: Radar,
  awaitMonitor: Radar,
  listRecordings: Clapperboard,
  readRecording: Clapperboard,
  injectCode: Code2,
  runCode: Braces,
};
