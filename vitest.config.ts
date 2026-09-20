import { defineConfig } from 'vitest/config';

/*
 * One vitest run for the repository. The app's project supplies jsdom, the
 * React plugin and its setup file; anything added later (a second package)
 * is another entry here.
 */
export default defineConfig({
  test: {
    projects: ['app/vitest.config.ts'],
  },
});
