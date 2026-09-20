import { fireEvent, render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { App } from '../../App.js';
import { createMockBackend, type MockBackend, type MockSeed } from '../../backend/mock.js';

const show = async (seed: MockSeed = {}) => {
  const mock = createMockBackend(seed);
  const user = userEvent.setup();
  render(<App backend={mock} />);
  const sidebar = await screen.findByRole('navigation', { name: 'Main' });
  await user.click(within(sidebar).getByRole('button', { name: 'Settings' }));
  await screen.findByRole('heading', { name: 'Settings', level: 1 });
  return { mock, user };
};

afterEach(() => {
  document.documentElement.removeAttribute('data-theme');
  localStorage.clear();
});

describe('the settings pane', () => {
  it('saves the device name when the field is left', async () => {
    const { mock, user } = await show();
    const rename = vi.spyOn(mock, 'setDeviceName');

    const field = screen.getByLabelText('Device name');
    await user.clear(field);
    await user.type(field, 'attic');
    await user.tab();

    expect(rename).toHaveBeenCalledWith('attic');
  });

  it('saves the device name on Enter without waiting for a blur', async () => {
    const { mock, user } = await show();
    const rename = vi.spyOn(mock, 'setDeviceName');

    const field = screen.getByLabelText('Device name');
    await user.clear(field);
    await user.type(field, 'attic{Enter}');

    expect(rename).toHaveBeenCalledWith('attic');
  });

  it('puts an emptied name back rather than saving nothing', async () => {
    const { mock, user } = await show();
    const rename = vi.spyOn(mock, 'setDeviceName');

    const field = screen.getByLabelText('Device name');
    await user.clear(field);
    await user.tab();

    expect(rename).not.toHaveBeenCalled();
    expect(field).toHaveValue('workshop');
  });

  it('moves the folder to whatever the picker returns', async () => {
    const { mock, user } = await show({ platform: 'desktop' });
    const move = vi.spyOn(mock, 'setFolder');

    await user.click(screen.getByRole('button', { name: 'Change' }));
    await vi.waitFor(() => expect(move).toHaveBeenCalledWith('/home/you/Shared'));

    const card = screen.getByRole('region', { name: 'Sync folder' });
    expect(await within(card).findByText('/home/you/Shared')).toBeInTheDocument();
  });

  it('offers to open the folder only where there is a file manager to open it in', async () => {
    await show({ platform: 'desktop' });
    expect(screen.getByRole('button', { name: 'Open folder' })).toBeInTheDocument();
  });

  it('leaves both folder buttons out on a phone', async () => {
    // Android's picker hands back a tree URI the engine cannot sync, so
    // `pickFolder` answers null and Change did nothing when it was pressed.
    await show({ platform: 'android' });
    expect(screen.queryByRole('button', { name: 'Open folder' })).toBeNull();
    expect(screen.queryByRole('button', { name: 'Change' })).toBeNull();
  });

  it('puts the chosen theme on the document and remembers it', async () => {
    const { user } = await show();
    const group = screen.getByRole('group', { name: 'Appearance' });

    await user.click(within(group).getByRole('button', { name: 'Dark' }));
    expect(document.documentElement).toHaveAttribute('data-theme', 'dark');
    expect(localStorage.getItem('owl-theme')).toBe('dark');

    await user.click(within(group).getByRole('button', { name: 'System' }));
    expect(document.documentElement).not.toHaveAttribute('data-theme');
    expect(localStorage.getItem('owl-theme')).toBe('system');
  });

  it('marks which of the three is in force', async () => {
    const { user } = await show();
    const group = screen.getByRole('group', { name: 'Appearance' });

    expect(within(group).getByRole('button', { name: 'System' })).toHaveAttribute(
      'aria-pressed',
      'true',
    );
    await user.click(within(group).getByRole('button', { name: 'Light' }));
    expect(within(group).getByRole('button', { name: 'Light' })).toHaveAttribute(
      'aria-pressed',
      'true',
    );
  });

  it('asks for storage access only on the platform that withholds it', async () => {
    await show({ platform: 'desktop' });
    expect(screen.queryByRole('region', { name: 'Storage access' })).toBeNull();
  });

  it('explains why sync is paused, and sends a person to the system screen', async () => {
    const { user } = await show({ platform: 'android', permission: 'denied' });

    const card = await screen.findByRole('region', { name: 'Storage access' });
    expect(card).toHaveTextContent('Syncing is paused until then.');

    await user.click(within(card).getByRole('button', { name: 'Open Android settings' }));
    expect(await within(card).findByText('Allowed')).toBeInTheDocument();
  });

  it('rechecks the permission when the window comes back from that screen', async () => {
    const { mock } = await show({ platform: 'android', permission: 'denied' });
    expect(await screen.findByText('Not allowed')).toBeInTheDocument();

    // Granted on the system screen, which this app never sees happen.
    await mock.openAllFilesSettings();
    fireEvent.focus(window);

    expect(await screen.findByText('Allowed')).toBeInTheDocument();
  });

  it('rechecks when the activity comes back, which is all a phone reports', async () => {
    // Android's WebView fires no focus event when its activity resumes. Left
    // to that alone the card would say "Not allowed" until the person pressed
    // something, with sync still paused behind it.
    const { mock } = await show({ platform: 'android', permission: 'denied' });
    expect(await screen.findByText('Not allowed')).toBeInTheDocument();

    await mock.openAllFilesSettings();
    fireEvent(document, new Event('visibilitychange'));

    expect(await screen.findByText('Allowed')).toBeInTheDocument();
  });

  it('names the version and offers the repository', async () => {
    const { mock, user } = await show();
    const open = vi.spyOn(mock, 'openUrl');

    const about = screen.getByRole('region', { name: 'About' });
    expect(about).toHaveTextContent('0.1.0');

    await user.click(within(about).getByRole('button', { name: /open source/ }));
    expect(open).toHaveBeenCalledWith('https://github.com/danieltyukov/owl-transfer');
  });
});
