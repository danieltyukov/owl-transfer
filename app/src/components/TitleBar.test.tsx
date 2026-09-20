import { act, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import { App } from '../App.js';
import { createMockBackend, createMockFrame } from '../backend/mock.js';

const frame = createMockFrame;

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
    const close = vi.spyOn(driven, 'close');
    const minimize = vi.spyOn(driven, 'minimize');
    render(<App backend={createMockBackend({ window: driven })} />);

    await user.click(await screen.findByRole('button', { name: 'Close' }));
    expect(close).toHaveBeenCalledOnce();

    await user.click(screen.getByRole('button', { name: 'Minimize' }));
    expect(minimize).toHaveBeenCalledOnce();

    await user.click(screen.getByRole('button', { name: 'Maximize' }));
    expect(driven.maximized).toBe(true);
  });

  it('says which way the middle control points, so it never names the wrong one', async () => {
    const user = userEvent.setup();
    const driven = frame();
    render(<App backend={createMockBackend({ window: driven })} />);

    await user.click(await screen.findByRole('button', { name: 'Maximize' }));
    expect(await screen.findByRole('button', { name: 'Restore' })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Maximize' })).toBeNull();

    await user.click(screen.getByRole('button', { name: 'Restore' }));
    expect(await screen.findByRole('button', { name: 'Maximize' })).toBeInTheDocument();
  });

  it('opens on the state the window already has, not on a guess', async () => {
    // A window restored from a maximised session is maximised before anything
    // in this app has run.
    render(<App backend={createMockBackend({ window: frame(true) })} />);
    expect(await screen.findByRole('button', { name: 'Restore' })).toBeInTheDocument();
  });

  it('follows the window when it is maximised by some other route', async () => {
    const driven = frame();
    render(<App backend={createMockBackend({ window: driven })} />);
    await screen.findByRole('button', { name: 'Maximize' });

    // A double press on the title bar, a keyboard shortcut, a drag to the top
    // of the screen: none of them come through this app's own button.
    act(() => driven.setMaximized(true));
    expect(await screen.findByRole('button', { name: 'Restore' })).toBeInTheDocument();
  });

  it('says so when the shell refuses, rather than losing the rejection', async () => {
    const user = userEvent.setup();
    const driven = frame();
    driven.close = () => Promise.reject(new Error('busy'));
    render(<App backend={createMockBackend({ window: driven })} />);

    await user.click(await screen.findByRole('button', { name: 'Close' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('The window would not close.');
  });

  it('names where the window is, and leaves the brand to one place', async () => {
    render(<App backend={createMockBackend({ window: frame() })} />);
    const bar = await screen.findByRole('banner');
    expect(bar).toHaveTextContent('OwlTransfer');
    expect(bar).toHaveTextContent('Files');
    // The sidebar's own brand would be the same word twice, eleven pixels apart.
    expect(screen.getAllByText('Owl', { exact: false }).length).toBeGreaterThan(0);
  });
});
