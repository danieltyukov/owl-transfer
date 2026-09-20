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
      // Not the mock: that would put a fictional folder in front of someone
      // whose real one is right there on disk. A sentence instead, because a
      // window that comes up empty and says nothing is the worst of the three.
      console.error('the shell could not be reached', error);
      root.render(
        <p role="alert" className="startup-error">
          OwlTransfer could not reach its own engine. Closing this window and
          opening it again is the thing to try.
        </p>,
      );
    });
} else {
  render(resolveBackend());
}
