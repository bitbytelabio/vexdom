import { isChimeKind, playTone } from '@/lib/chime/tone';

/* The offscreen page a Chromium background plays a chime through; the background closes it afterwards. */
const kind = new URLSearchParams(location.search).get('kind');
if (isChimeKind(kind)) void playTone(kind);
