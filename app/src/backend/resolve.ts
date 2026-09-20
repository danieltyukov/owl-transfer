import { createMockBackend, createMockFrame } from './mock.js';
import type { Backend } from './types.js';

/*
 * The backend for everywhere there is no shell.
 *
 * That is a browser during development, the Playwright smoke, and nothing else:
 * under Tauri, `main.tsx` imports the adapter instead and never calls this.
 * Nothing here imports Tauri, which is what keeps every one of its modules out
 * of the browser bundle and out of the tests.
 *
 * So this is the mock, and the only decision left is which mock.
 */

export function resolveBackend(): Backend {
  // `?frame` draws the title bar the desktop window gets, so the chrome can be
  // looked at without building the shell. `?android` does the same for the
  // permission card. Both are development switches and neither is reachable
  // from the interface.
  const query = typeof location === 'undefined' ? '' : location.search;
  const params = new URLSearchParams(query);
  return createMockBackend({
    // A frame with no window behind it, for looking at the desktop chrome
    // in a browser tab.
    ...(params.has('frame') ? { window: createMockFrame() } : {}),
    ...(params.has('android') ? { platform: 'android' as const, permission: 'denied' as const } : {}),
  });
}
