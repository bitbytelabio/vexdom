import '@/lib/zod-locale';
import { exposeActions } from '@/lib/actions/host';
import { exposeCues } from '@/lib/cues/host';
import { exposeFrameProbe } from '@/lib/frames/host';
import { exposeHandsFree } from '@/lib/handsfree/host';
import { exposeIndicator } from '@/lib/indicator/host';
import { exposeMonitor } from '@/lib/monitor/watch';
import { exposeRail } from '@/lib/rail/host';
import { exposeRecorder } from '@/lib/recordings/host';
import { exposeRegionPicker } from '@/lib/region/host';
import { exposeToast } from '@/lib/toast/host';

export default defineContentScript({
  matches: ['*://*/*'],
  main() {
    const world = globalThis as { __vexdomActions?: boolean };
    if (world.__vexdomActions) return;
    world.__vexdomActions = true;
    exposeActions();
    exposeFrameProbe();
    exposeCues();
    if (window !== window.top) return;
    exposeRecorder();
    exposeMonitor();
    exposeIndicator();
    exposeRail();
    exposeToast();
    exposeHandsFree();
    exposeRegionPicker();
  },
});
