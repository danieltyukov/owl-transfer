import { act, fireEvent, render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import { App } from './App.js';
import { createMockBackend, type MockBackend, type MockSeed } from './backend/mock.js';

/*
 * The first run on Android, where the engine is paused because the sync folder
 * is in shared storage and all files access has not been granted yet.
 *
 * Every one of these is about a screen other than Settings. The card there was
 * the only thing that knew about the permission, so a person who never opened
 * Settings saw an empty folder, a strip that said nothing was paired, and an
 * Add files button that failed without a reason.
 */
const phone = (seed: MockSeed = {}): MockBackend =>
  createMockBackend({
    platform: 'android',
    permission: 'denied',
    entries: [],
    ...seed,
    state: {
      folder: '/storage/emulated/0/OwlTransfer',
      paused: true,
      peers: [],
      nearby: [],
      transfers: { active: [], queued: 0, bytesPerSec: 0 },
      ...seed.state,
    },
  });

/** The one sentence every refused action gives, and the card's own heading. */
const REASON = 'Sync is paused until storage access is allowed, so nothing can be added yet.';

const storageCard = () => screen.findByRole('region', { name: 'Storage access' });

/*
 * The header and the empty folder both offer Add files and New folder, and the
 * two go to the same handler. The header's is the one a person has on every
 * screen, so it is the one pressed here.
 */
const tool = (name: string): HTMLElement => screen.getAllByRole('button', { name })[0]!;

describe('a phone waiting for storage access', () => {
  it('explains itself on the Files screen rather than only in Settings', async () => {
    render(<App backend={phone()} />);

    // The Files pane, which is what a fresh install opens on.
    expect(await screen.findByRole('heading', { name: 'OwlTransfer' })).toBeInTheDocument();

    const card = await storageCard();
    expect(card).toHaveTextContent('Syncing is paused until then.');
    expect(within(card).getByRole('button', { name: 'Open Android settings' })).toBeInTheDocument();
  });

  it('says on the status strip that sync is paused, not that it is catching up', async () => {
    // A peer connected and the folder behind, which is the state that used to
    // read "Catching up" for as long as the permission was withheld.
    const mock = phone();
    render(<App backend={mock} />);
    await storageCard();

    mock.emitState({
      peers: [
        {
          id: '9f2c71aa4d38e5b0',
          name: 'workshop',
          kind: 'desktop',
          connected: true,
          address: '192.168.1.20:52734',
          lastSeenMs: Date.now(),
          pairedAtMs: Date.now(),
        },
      ],
    });

    const strip = await screen.findByRole('status');
    expect(strip).toHaveTextContent('Sync paused until storage access is allowed');
    expect(strip).not.toHaveTextContent('Catching up');
  });

  it('gives Add files a reason instead of a bare failure', async () => {
    const user = userEvent.setup();
    const mock = phone();
    const importing = vi.spyOn(mock, 'pickAndImport');
    render(<App backend={mock} />);
    await storageCard();

    await user.click(tool('Add files'));

    expect(await screen.findByRole('alert')).toHaveTextContent(REASON);
    expect(importing).not.toHaveBeenCalled();
  });

  it('gives a dropped or shared file the same reason', async () => {
    const mock = phone();
    const importing = vi.spyOn(mock, 'importPaths');
    render(<App backend={mock} />);
    await storageCard();

    act(() => mock.emitDrop(['/storage/emulated/0/Pictures/kestrel.png']));

    expect(await screen.findByRole('alert')).toHaveTextContent(REASON);
    expect(importing).not.toHaveBeenCalled();
  });

  it('gives New folder the same reason', async () => {
    const user = userEvent.setup();
    const mock = phone();
    const creating = vi.spyOn(mock, 'createFolder');
    render(<App backend={mock} />);
    await storageCard();

    await user.click(tool('New folder'));
    await user.type(screen.getByLabelText('Name'), 'Photos');
    await user.click(screen.getByRole('button', { name: 'Create' }));

    expect(await screen.findByRole('alert')).toHaveTextContent(REASON);
    expect(creating).not.toHaveBeenCalled();
  });

  it('starts the engine when the activity comes back from the system screen', async () => {
    const mock = phone();
    const resume = vi.spyOn(mock, 'setPaused');
    render(<App backend={mock} />);
    await storageCard();

    // Granted on Android's own screen, which this app never sees happen. The
    // WebView fires no focus event when its activity resumes, only this.
    await mock.openAllFilesSettings();
    fireEvent(document, new Event('visibilitychange'));

    await vi.waitFor(() => expect(resume).toHaveBeenCalledWith(false));
    expect(screen.queryByRole('region', { name: 'Storage access' })).toBeNull();
    expect(await screen.findByRole('status')).not.toHaveTextContent('paused');
  });

  it('keeps asking while it waits, for a grant that fires no event at all', async () => {
    vi.useFakeTimers();
    try {
      const mock = phone();
      const resume = vi.spyOn(mock, 'setPaused');
      render(<App backend={mock} />);
      await act(async () => {
        await vi.advanceTimersByTimeAsync(0);
      });
      expect(screen.getByRole('region', { name: 'Storage access' })).toBeInTheDocument();

      // Allowed from Android's settings with this app never hidden: no focus,
      // no visibility change, nothing for the two listeners to hear.
      await mock.openAllFilesSettings();
      expect(resume).not.toHaveBeenCalled();

      await act(async () => {
        await vi.advanceTimersByTimeAsync(3_000);
      });

      expect(resume).toHaveBeenCalledWith(false);
      expect(screen.queryByRole('region', { name: 'Storage access' })).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it('lets Add files through once storage is allowed', async () => {
    const user = userEvent.setup();
    const mock = phone();
    const importing = vi.spyOn(mock, 'pickAndImport');
    render(<App backend={mock} />);

    const card = await storageCard();
    await user.click(within(card).getByRole('button', { name: 'Open Android settings' }));
    await vi.waitFor(() =>
      expect(screen.queryByRole('region', { name: 'Storage access' })).toBeNull(),
    );

    await user.click(tool('Add files'));
    expect(importing).toHaveBeenCalledWith('');
    expect(screen.queryByRole('alert')).toBeNull();
  });

  it('keeps the card in Settings, where it says the permission is allowed', async () => {
    const user = userEvent.setup();
    const mock = phone({ permission: 'granted', state: { paused: false } });
    render(<App backend={mock} />);

    const tabs = await screen.findByRole('navigation', { name: 'Sections' });
    await user.click(within(tabs).getByRole('button', { name: 'Settings' }));

    const card = await storageCard();
    expect(card).toHaveTextContent('Allowed');
  });

  it('puts no storage card on a screen that is not waiting for one', async () => {
    // A desktop, where the permission does not apply and nothing is paused.
    render(<App backend={createMockBackend({ platform: 'desktop' })} />);
    await screen.findByRole('heading', { name: 'OwlTransfer' });

    expect(screen.queryByRole('region', { name: 'Storage access' })).toBeNull();
    expect(await screen.findByRole('status')).not.toHaveTextContent('paused');
  });
});
