import { defineConfig } from 'wxt';
import tailwindcss from '@tailwindcss/vite';
import { manifestCommands } from './src/lib/settings/shortcuts';

export default defineConfig({
  modules: ['@wxt-dev/module-react'],

  // srcDir is `src`, not `src/extension`, because WXT points its `@` alias at srcDir and the
  // shared code lives in `src/lib` (the extension is the only TypeScript project; the Bridge is
  // Rust). Aiming `@` at src keeps `@/lib/...` resolving. The extension's own files then live
  // one level down, which is why entrypoints and public are named explicitly.
  srcDir: 'src',
  // entrypointsDir resolves against srcDir; publicDir resolves against the project root.
  // They look inconsistent because they are, so both are written out in full here rather
  // than relying on memory. Getting publicDir wrong silently ships an extension with no
  // icons, because WXT discovers those from public/icon/*.png.
  entrypointsDir: 'extension/entrypoints',
  publicDir: 'src/extension/public',

  outDir: 'dist',
  vite: () => ({
    plugins: [tailwindcss()],
  }),
  hooks: {
    // WXT writes sidebar_action without an icon, so Firefox's sidebar launcher shows a puzzle piece.
    'build:manifestGenerated': (_wxt, manifest) => {
      if (manifest.sidebar_action) {
        manifest.sidebar_action.default_icon = {
          16: 'icon/16.png',
          32: 'icon/32.png',
          48: 'icon/48.png',
          96: 'icon/96.png',
        };
      }
    },
  },
  manifest: ({ browser }) => ({
    name: 'Vexdom',
    description:
      'Your real, signed-in browser, driven by Claude Code or any MCP client. Runs on this computer: no account, no cloud.',
    permissions: [
      'storage',
      'unlimitedStorage',
      'activeTab',
      'contextMenus',
      'alarms',
      'scripting',
      'notifications',
      'downloads',
      'nativeMessaging',
      'cookies',
      'clipboardRead',
      'clipboardWrite',
      // Each conversation's tabs sit in a tab group of their own; Firefox has had the API since 139.
      'tabGroups',
      // An action's after-action snapshot waits for the data requests it started (read-only, no
      // new warning: <all_urls> already covers the sites).
      'webRequest',
      // Firefox has no sidePanel, debugger or offscreen permission, and addons.mozilla.org flags each name it does not know.
      // offscreen is the hidden page hands-free mode listens through once the side panel is closed.
      // userScripts runs a saved tool on every visit, once the user turns on Allow User Scripts.
      // topSites and favicon fill the new tab page's frequent sites; <all_urls> already covers both warnings in Chrome.
      ...(browser === 'firefox' ? [] : ['sidePanel', 'debugger', 'offscreen', 'userScripts', 'topSites', 'favicon']),
    ],
    // Firefox shows topSites as a new warning, which would hold every update until accepted, so the new tab page asks for it.
    ...(browser === 'firefox' && { optional_permissions: ['topSites'] }),
    host_permissions: ['<all_urls>'],
    // WXT writes the toolbar key only for a popup entrypoint, and the toolbar button opens the side panel instead.
    action: { default_title: 'Vexdom' },
    // ext-core, the Bridge link, is wasm (ADR-017): compiling it needs 'wasm-unsafe-eval', which
    // Chrome takes from 116 on. Firefox's MV2 manifest takes the policy as a plain string.
    content_security_policy:
      browser === 'firefox'
        ? ("script-src 'self' 'wasm-unsafe-eval'; object-src 'self'" as never)
        : { extension_pages: "script-src 'self' 'wasm-unsafe-eval'; object-src 'self'" },
    ...(browser !== 'firefox' && { minimum_chrome_version: '116' }),
    commands: manifestCommands(browser === 'firefox'),
    // Release Firefox installs only what addons.mozilla.org has signed, and signing needs a
    // permanent id. The update URL is polled daily; the release job publishes updates.json
    // next to each signed .xpi, and `releases/latest` always resolves to the newest one.
    // websiteContent, because what the agent reads on a page reaches the model behind it.
    ...(browser === 'firefox' && {
      browser_specific_settings: {
        gecko: {
          id: 'vexdom@nguyenthdat.github.io',
          strict_min_version: '140.0',
          update_url: 'https://github.com/bitbytelabio/vexdom/releases/latest/download/updates.json',
          data_collection_permissions: { required: ['websiteContent'] },
        },
      },
    }),
  }),
});
