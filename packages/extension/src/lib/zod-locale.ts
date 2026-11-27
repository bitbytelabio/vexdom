import { z } from 'zod';

// zod marks itself side-effect free, so bundling drops its own en locale and every error reads "Invalid input".
z.config(z.locales.en());
