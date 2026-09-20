import react from '@vitejs/plugin-react';
import { defineConfig } from 'vite';

// The Tauri CLI sets TAURI_DEV_HOST when the dev server has to be reachable
// from a phone or an emulator; on the desktop it is unset and the server binds
// to localhost only.
const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    ...(host ? { hmr: { protocol: 'ws', host, port: 1421 } } : {}),
    watch: { ignored: ['**/src-tauri/**'] },
  },
  envPrefix: ['VITE_', 'TAURI_ENV_*'],
  build: {
    minify: process.env.TAURI_ENV_DEBUG ? false : 'esbuild',
    sourcemap: Boolean(process.env.TAURI_ENV_DEBUG),
  },
});
