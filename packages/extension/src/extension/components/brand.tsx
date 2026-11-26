import { cn } from '@/lib/utils';

/** The vexdom mark: an X whose upper half is the V, in a HUD reticle. `drawn` traces the V, then drops the legs and hub in, once, as the mark first appears. */
export function Logo({ className, drawn = false }: { className?: string; drawn?: boolean }) {
  return (
    <svg viewBox="0 0 32 32" fill="none" className={cn('size-7', drawn && 'logo-drawn', className)} aria-hidden>
      <path
        d="M2 7.5V2h5.5M24.5 2H30v5.5M30 24.5V30h-5.5M7.5 30H2v-5.5"
        stroke="currentColor"
        strokeOpacity="0.45"
        strokeWidth="1.5"
      />
      <path
        className="logo-legs"
        d="M16 18.5L22.3 26.6M16 18.5L9.7 26.6"
        stroke="var(--magenta)"
        strokeWidth="2.25"
        strokeLinecap="square"
      />
      <path
        className="logo-v"
        d="M7 7L16 18.5L25 7"
        pathLength={1}
        stroke="currentColor"
        strokeWidth="3.25"
        strokeLinecap="square"
        strokeLinejoin="miter"
      />
      <rect
        className="logo-hub"
        x="14.4"
        y="16.9"
        width="3.2"
        height="3.2"
        fill="var(--ground)"
        stroke="currentColor"
        strokeWidth="1.4"
      />
    </svg>
  );
}

export function Wordmark({ className }: { className?: string }) {
  return (
    <span className={cn('flex min-w-0 items-center gap-2', className)}>
      <Logo className="size-6 shrink-0 text-brand" />
      <span className="truncate font-display text-[0.8125rem] leading-none font-semibold tracking-[0.14em] uppercase text-ink max-[19rem]:hidden">
        Vexdom
      </span>
    </span>
  );
}
