import { defineConfig, devices } from '@playwright/test';

/*
 * The smoke test, which is the only check that runs against a real browser.
 *
 * jsdom cannot answer the one question the phone layout raises: whether the
 * tabs and the sidebar swap at the breakpoint. It applies no CSS, so a media
 * query means nothing to it. This builds the app, serves it, and looks.
 *
 * The app runs on the mock backend here, the same way it does in a browser
 * during development: `__TAURI_INTERNALS__` is not on the window, so `main.tsx`
 * never imports the adapter and `resolveBackend` hands back the mock.
 */
export default defineConfig({
  testDir: './e2e',
  outputDir: './e2e/results',
  fullyParallel: true,
  forbidOnly: Boolean(process.env.CI),
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? 'line' : 'list',
  use: {
    baseURL: 'http://127.0.0.1:4173',
    trace: 'retain-on-failure',
  },
  projects: [
    {
      name: 'desktop',
      use: { ...devices['Desktop Chrome'], viewport: { width: 1280, height: 800 } },
    },
    {
      /*
       * A real touch device, not a narrow desktop window.
       *
       * The viewport alone makes the pane layout swap, but `pointer: coarse`
       * never matches without touch, so the 44px row, the always-visible
       * overflow control, the 40px menu items and the 36px inputs would all be
       * drawn at their desktop sizes in every screenshot. The device
       * descriptor brings touch and the mobile user agent; the viewport is
       * overridden to the 390x844 the plan names.
       */
      name: 'phone',
      use: { ...devices['Pixel 7'], viewport: { width: 390, height: 844 } },
    },
  ],
  webServer: {
    command: 'npm run build && npm run preview -- --port 4173 --strictPort --host 127.0.0.1',
    url: 'http://127.0.0.1:4173',
    reuseExistingServer: !process.env.CI,
    timeout: 120_000,
  },
});
