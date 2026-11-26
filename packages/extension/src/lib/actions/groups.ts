export interface ToolGroup {
  /** Its line in the index, in the words someone would use for the need. */
  readonly index: string;
  /** Bundled skills whose bodies open the group's guide, in order. */
  readonly skills: readonly string[];
}

export const TOOL_GROUPS = {
  captcha: { index: 'get past a “verify you are human” check: reCAPTCHA, hCaptcha, Turnstile', skills: ['captcha'] },
  theming: { index: 'measure the colours and contrast a page paints, and retheme it: dark mode, easier on the eyes', skills: ['page-theming'] },
  diagnostics: { index: 'read the console errors, exceptions and failed requests behind a page that misbehaves', skills: ['page-diagnostics'] },
  monitoring: {
    index: 'watch a long upload, build or deploy and tell the user when it ends, or come back to something later on a timer',
    skills: ['monitor-progress', 'scheduled-jobs'],
  },
  downloads: { index: 'make the page download a file and keep it, and list the files kept so far', skills: ['downloads'] },
  drag: { index: 'drag one thing onto another: reorder a list, move a card, pull a slider, draw on a canvas', skills: ['drag'] },
  storage: { index: 'read or change cookies, localStorage and sessionStorage, and the clipboard', skills: ['page-storage'] },
  navigation: {
    index: 'change an app’s route without a reload, open a new window, or wait for text, a URL, the load or a condition',
    skills: ['navigation'],
  },
  input: { index: 'insert text without key events', skills: ['input'] },
  code: { index: 'run JavaScript in the page and get its value back', skills: ['code'] },
  network: { index: 'mock or block requests, record a HAR, read requests and their bodies', skills: ['network'] },
  dialogs: { index: 'see, accept or dismiss alert, confirm, prompt and leave-page dialogs', skills: ['dialogs'] },
  emulation: { index: 'pretend to be another screen, phone, place, network or colour scheme', skills: ['emulation'] },
  capture: { index: 'save the page as a PDF or record a video of the tab', skills: ['capture'] },
  devtools: {
    index: 'trace or profile the page, measure Core Web Vitals, inspect React, audit accessibility',
    skills: ['devtools'],
  },
} as const satisfies Record<string, ToolGroup>;

export type ToolGroupName = keyof typeof TOOL_GROUPS;
