import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import { App } from '../App.js';
import { createMockBackend } from '../backend/mock.js';
import type { WindowFrame } from '../backend/types.js';

const frame = (): WindowFrame => ({
  minimize: vi.fn(() => Promise.resolve()),
  toggleMaximize: vi.fn(() => Promise.resolve()),
  close: vi.fn(() => Promise.resolve()),
  startDrag: vi.fn(() => Promise.resolve()),
});

describe('TitleBar', () => {
  it('is drawn only for a window the shell left undecorated', async () => {
    render(<App backend={createMockBackend()} />);
    await screen.findByRole('status');
    // A browser tab and an Android activity have chrome of their own; a second
    // close button inside the page would close the wrong thing.
    expect(screen.queryByRole('group', { name: 'Window' })).toBeNull();
  });

  it('carries the window controls when there is a frame to drive', async () => {
    render(<App backend={createMockBackend({ window: frame() })} />);
    const controls = await screen.findByRole('group', { name: 'Window' });
    expect(controls).toBeInTheDocument();
  });

  it('sends each control straight back to the shell', async () => {
    const user = userEvent.setup();
    const driven = frame();
    render(<App backend={createMockBackend({ window: driven })} />);

    await user.click(await screen.findByRole('button', { name: 'Close' }));
    expect(driven.close).toHaveBeenCalledOnce();

    await user.click(screen.getByRole('button', { name: 'Minimize' }));
    expect(driven.minimize).toHaveBeenCalledOnce();

    await user.click(screen.getByRole('button', { name: 'Maximize or restore' }));
    expect(driven.toggleMaximize).toHaveBeenCalledOnce();
  });

  it('names where the window is, and leaves the brand to one place', async () => {
    render(<App backend={createMockBackend({ window: frame() })} />);
    const bar = await screen.findByRole('banner');
    expect(bar).toHaveTextContent('Owl Transfer');
    expect(bar).toHaveTextContent('Files');
    // The sidebar's own brand would be the same word twice, eleven pixels apart.
    expect(screen.getAllByText('Owl', { exact: false }).length).toBeGreaterThan(0);
  });
});
