import { createMockBackend } from './mock.js';
import type { Backend } from './types.js';

/*
 * Which backend the interface is running on.
 *
 * The shell sets `window.__owlBackend` before the bundle loads, so by the time
 * this runs the answer is already there. Nothing here imports Tauri: the
 * adapter that does lives in the shell's own module, which a browser never
 * downloads and a test never has to stub.
 *
 * Everywhere else there is no engine, so the interface runs on the mock. That
 * is the browser during development and the Playwright smoke; it is also what
 * makes `npm run dev` useful without a Rust toolchain.
 */

declare global {
  interface Window {
    __owlBackend?: Backend;
  }
}

/** A frame whose buttons do nothing, for previewing the desktop chrome in a tab. */
const previewFrame = {
  minimize: () => Promise.resolve(),
  toggleMaximize: () => Promise.resolve(),
  close: () => Promise.resolve(),
  startDrag: () => Promise.resolve(),
};

export function resolveBackend(): Backend {
  const shell = typeof window === 'undefined' ? undefined : window.__owlBackend;
  if (shell !== undefined) return shell;

  // `?frame` draws the title bar the desktop window gets, so the chrome can be
  // looked at without building the shell. `?android` does the same for the
  // permission card. Both are development switches and neither is reachable
  // from the interface.
  const query = typeof location === 'undefined' ? '' : location.search;
  const params = new URLSearchParams(query);
  return createMockBackend({
    ...(params.has('frame') ? { window: previewFrame } : {}),
    ...(params.has('android') ? { platform: 'android' as const, permission: 'denied' as const } : {}),
  });
}
