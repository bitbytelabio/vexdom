import { configDefaults, defineConfig } from 'vitest/config';
import { WxtVitest } from 'wxt/testing/vitest-plugin';
import { COVERAGE_AREAS, CoverageByArea } from './vitest.coverage';

export default defineConfig({
  test: {
    reporters: ['default', new CoverageByArea()],
    projects: [
      {
        plugins: [WxtVitest()],
        test: {
          name: 'lib',
          include: ['src/lib/**/*.test.ts'],
          setupFiles: ['src/lib/test/load-core.ts'],
          exclude: [
            ...configDefaults.exclude,
            'src/lib/**/*.integration.test.ts',
            'src/lib/actions/page/**',
            'src/lib/handsfree/host.test.ts',
            'src/lib/cues/host.test.ts',
          ],
        },
      },
      {
        // The extension against the Rust Bridge binary and ext-core's node build (src/lib/bridge/test/rust-bridge.ts).
        plugins: [WxtVitest()],
        test: {
          name: 'link',
          include: ['src/lib/**/*.integration.test.ts'],
          setupFiles: ['src/lib/test/load-core.ts'],
          testTimeout: 20_000,
        },
      },
      {
        plugins: [WxtVitest()],
        test: {
          name: 'dom',
          environment: 'happy-dom',
          setupFiles: ['src/lib/test/load-core.ts'],
          include: ['src/lib/actions/page/**/*.test.ts', 'src/lib/handsfree/host.test.ts', 'src/lib/cues/host.test.ts'],
        },
      },
    ],
    coverage: {
      provider: 'v8',
      include: ['src/**/*.{ts,tsx}'],
      exclude: ['src/**/*.test.ts', 'src/**/*.test-d.ts', 'src/**/*.d.ts', 'src/**/fixtures/**', 'src/lib/bridge/ext-core/**', 'src/lib/bridge/test/**', 'src/lib/test/**'],
      reporter: ['json-summary', 'html'],
      thresholds: Object.fromEntries(COVERAGE_AREAS.map(({ glob, lines }) => [glob, { lines }])),
    },
  },
});
