import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';

import { App } from './App.js';
import { createMockBackend, type MockBackend } from './backend/mock.js';

const quiet = (mock: MockBackend): void => {
  mock.emitState({
    transfers: { active: [], queued: 0, bytesPerSec: 0 },
    summary: { ...mock.state.summary, upToDate: true },
  });
};

describe('App', () => {
  it('opens on the folder, with the sidebar beside it', async () => {
    const mock = createMockBackend();
    render(<App backend={mock} />);

    const sidebar = await screen.findByRole('navigation', { name: 'Main' });
    expect(within(sidebar).getByRole('button', { name: 'Files' })).toBeInTheDocument();
    expect(within(sidebar).getByRole('button', { name: /Pixel/ })).toBeInTheDocument();

    // The last crumb is the heading, and it is the sync folder's own name.
    expect(await screen.findByRole('heading', { name: 'OwlTransfer' })).toBeInTheDocument();
    expect(await screen.findByRole('button', { name: /^kestrel\.png/ })).toBeInTheDocument();
  });

  it('says the folder is up to date once nothing is moving', async () => {
    const mock = createMockBackend();
    render(<App backend={mock} />);
    await screen.findByRole('status');

    quiet(mock);
    expect(await screen.findByRole('status')).toHaveTextContent('Up to date');
  });

  it('counts what is in flight while files are moving', async () => {
    const mock = createMockBackend();
    render(<App backend={mock} />);

    const strip = await screen.findByRole('status');
    expect(strip).toHaveTextContent('Syncing 1 file');
    expect(strip).toHaveTextContent('12 MB/s');
  });

  it('distinguishes a peer that is asleep from having no peer at all', async () => {
    const mock = createMockBackend();
    render(<App backend={mock} />);
    await screen.findByRole('status');

    quiet(mock);
    const peer = mock.state.peers[0]!;
    mock.emitState({ peers: [{ ...peer, connected: false }] });
    expect(await screen.findByRole('status')).toHaveTextContent('No device connected');

    mock.emitState({ peers: [] });
    expect(await screen.findByRole('status')).toHaveTextContent('No device paired');
  });

  it('walks into a folder and back out by its crumb', async () => {
    const user = userEvent.setup();
    const mock = createMockBackend();
    render(<App backend={mock} />);

    await user.click(await screen.findByRole('button', { name: /^Photos/ }));
    expect(await screen.findByRole('heading', { name: 'Photos' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /^lighthouse\.jpg/ })).toBeInTheDocument();

    await user.click(screen.getByRole('button', { name: 'OwlTransfer' }));
    expect(await screen.findByRole('heading', { name: 'OwlTransfer' })).toBeInTheDocument();
  });

  it('keeps the folder it was showing when a pane is visited and left', async () => {
    const user = userEvent.setup();
    const mock = createMockBackend();
    render(<App backend={mock} />);

    await user.click(await screen.findByRole('button', { name: /^Photos/ }));
    await screen.findByRole('heading', { name: 'Photos' });

    const sidebar = screen.getByRole('navigation', { name: 'Main' });
    await user.click(within(sidebar).getByRole('button', { name: 'Settings' }));
    await screen.findByRole('heading', { name: 'Settings' });

    await user.click(within(sidebar).getByRole('button', { name: 'Files' }));
    expect(await screen.findByRole('heading', { name: 'Photos' })).toBeInTheDocument();
  });

  it('offers the same three places in the tabs a phone uses', async () => {
    const mock = createMockBackend();
    render(<App backend={mock} />);

    const tabs = await screen.findByRole('navigation', { name: 'Sections' });
    expect(within(tabs).getAllByRole('button').map(b => b.textContent)).toEqual([
      'Files',
      'Devices',
      'Settings',
    ]);
  });
});
