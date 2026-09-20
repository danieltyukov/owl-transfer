import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';

import { App } from './App.js';
import { resolveBackend } from './backend/resolve.js';
import type { Backend } from './backend/types.js';
import { applyTheme, readTheme } from './theme.js';
import './base.css';

// Before the first paint, so a dark theme does not arrive as a flash of light.
applyTheme(readTheme());

const container = document.getElementById('root');
if (!container) {
  throw new Error('root element not found');
}
const root = createRoot(container);

function render(backend: Backend): void {
  root.render(
    <StrictMode>
      <App backend={backend} />
    </StrictMode>,
  );
}

/*
 * Which backend, and where it comes from.
 *
 * `__TAURI_INTERNALS__` is put on the window by the shell before the bundle
 * runs, so it is there by now if it is ever going to be. The adapter is
 * imported rather than bundled, which keeps every Tauri module out of the
 * browser build: `npm run dev` and the tests load none of it and need no stub.
 */
if ('__TAURI_INTERNALS__' in window) {
  void import('./backend/tauri.js')
    .then(module => module.createTauriBackend())
    .then(render, error => {
      // Falling back to the mock here would put a fictional folder in front of
      // someone whose real one is right there on disk.
      console.error('the shell could not be reached', error);
    });
} else {
  render(resolveBackend());
}
