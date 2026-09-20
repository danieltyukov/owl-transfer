import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';

import { App } from './App.js';
import { resolveBackend } from './backend/resolve.js';
import { applyTheme, readTheme } from './theme.js';
import './base.css';

// Before the first paint, so a dark theme does not arrive as a flash of light.
applyTheme(readTheme());

const container = document.getElementById('root');
if (!container) {
  throw new Error('root element not found');
}

createRoot(container).render(
  <StrictMode>
    <App backend={resolveBackend()} />
  </StrictMode>,
);
