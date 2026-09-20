import { fireEvent, render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import { App } from '../../App.js';
import { createMockBackend } from '../../backend/mock.js';
import { folderName, nameProblem, sortEntries } from './Files.js';

const rowNames = (): string[] =>
  screen
    .getAllByRole('button', { name: /^More for / })
    .map(button => button.getAttribute('aria-label')!.replace('More for ', ''));

const openMenuFor = async (user: ReturnType<typeof userEvent.setup>, name: string) => {
  await user.click(await screen.findByRole('button', { name: `More for ${name}` }));
  return screen.getByRole('menu', { name });
};

describe('sortEntries', () => {
  it('puts folders first and then counts the way a person does', () => {
    const entry = (name: string, isDir: boolean) => ({
      name,
      path: name,
      isDir,
      size: 0,
      mtimeMs: 0,
      status: { kind: 'synced' as const },
    });
    const sorted = sortEntries([
      entry('take 10.wav', false),
      entry('take 9.wav', false),
      entry('Notes', true),
    ]);
    expect(sorted.map(e => e.name)).toEqual(['Notes', 'take 9.wav', 'take 10.wav']);
  });
});

describe('folderName', () => {
  it('reads the last segment of either kind of path', () => {
    expect(folderName('/home/you/OwlTransfer')).toBe('OwlTransfer');
    expect(folderName('C:\\Users\\you\\OwlTransfer')).toBe('OwlTransfer');
    expect(folderName('/storage/emulated/0/OwlTransfer/')).toBe('OwlTransfer');
  });
});

describe('nameProblem', () => {
  it('turns away the names a file cannot have', () => {
    expect(nameProblem('Photos')).toBeNull();
    expect(nameProblem('   ')).toBe('Give it a name.');
    expect(nameProblem('..')).toBe('That name is reserved.');
    expect(nameProblem('a/b')).toBe('A name cannot contain a slash.');
    expect(nameProblem('a\\b')).toBe('A name cannot contain a slash.');
  });
});

describe('the files pane', () => {
  it('lists folders before files', async () => {
    render(<App backend={createMockBackend()} />);
    await screen.findByRole('button', { name: /^readme\.txt/ });
    expect(rowNames()).toEqual([
      'Photos',
      'Recordings',
      'kestrel.png',
      'owl calls.zip',
      'readme.txt',
    ]);
  });

  it('says where each entry has got to', async () => {
    const user = userEvent.setup();
    render(<App backend={createMockBackend()} />);

    expect(await screen.findByRole('button', { name: /^readme\.txt/ })).toHaveAccessibleName(
      /Synced/,
    );
    expect(screen.getByRole('button', { name: /^kestrel\.png/ })).toHaveAccessibleName(
      /Syncing, 42 percent/,
    );
    expect(screen.getByRole('button', { name: /^owl calls\.zip/ })).toHaveAccessibleName(
      /Only here/,
    );

    await user.click(screen.getByRole('button', { name: /^Recordings/ }));
    expect(await screen.findByRole('button', { name: /^barn owl 4am\.wav/ })).toHaveAccessibleName(
      /Waiting/,
    );
    expect(screen.getByRole('button', { name: /^field notes\.md/ })).toHaveAccessibleName(
      /Conflict/,
    );
  });

  it('creates a folder from the dialog and shows it in the list', async () => {
    const user = userEvent.setup();
    render(<App backend={createMockBackend()} />);
    await screen.findByRole('button', { name: /^readme\.txt/ });

    await user.click(screen.getByRole('button', { name: 'New folder' }));
    const dialog = screen.getByRole('dialog', { name: 'New folder' });
    await user.type(within(dialog).getByLabelText('Name'), 'Scans');
    await user.click(within(dialog).getByRole('button', { name: 'Create' }));

    expect(await screen.findByRole('button', { name: /^Scans/ })).toBeInTheDocument();
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('will not create a folder whose name a file cannot have', async () => {
    const user = userEvent.setup();
    render(<App backend={createMockBackend()} />);
    await screen.findByRole('button', { name: /^readme\.txt/ });

    await user.click(screen.getByRole('button', { name: 'New folder' }));
    const dialog = screen.getByRole('dialog', { name: 'New folder' });
    expect(within(dialog).getByRole('button', { name: 'Create' })).toBeDisabled();

    await user.type(within(dialog).getByLabelText('Name'), 'a/b');
    expect(within(dialog).getByText('A name cannot contain a slash.')).toBeInTheDocument();
    expect(within(dialog).getByRole('button', { name: 'Create' })).toBeDisabled();
  });

  it('renames through the row menu', async () => {
    const user = userEvent.setup();
    render(<App backend={createMockBackend()} />);
    await screen.findByRole('button', { name: /^readme\.txt/ });

    const menu = await openMenuFor(user, 'readme.txt');
    await user.click(within(menu).getByRole('menuitem', { name: 'Rename' }));

    const dialog = screen.getByRole('dialog', { name: 'Rename readme.txt' });
    const field = within(dialog).getByLabelText('Name');
    expect(field).toHaveValue('readme.txt');
    await user.clear(field);
    await user.type(field, 'read me first.txt');
    await user.click(within(dialog).getByRole('button', { name: 'Rename' }));

    expect(await screen.findByRole('button', { name: /^read me first\.txt/ })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /^readme\.txt/ })).toBeNull();
  });

  it('asks before deleting, and says what a folder takes with it', async () => {
    const user = userEvent.setup();
    render(<App backend={createMockBackend()} />);
    await screen.findByRole('button', { name: /^Photos/ });

    const menu = await openMenuFor(user, 'Photos');
    await user.click(within(menu).getByRole('menuitem', { name: 'Delete' }));

    const dialog = screen.getByRole('dialog', { name: 'Delete Photos?' });
    expect(dialog).toHaveTextContent('The folder and everything in it goes from both devices.');

    await user.click(within(dialog).getByRole('button', { name: 'Delete' }));
    await vi.waitFor(() => expect(screen.queryByRole('button', { name: /^Photos/ })).toBeNull());
  });

  it('leaves the entry alone when the delete is cancelled', async () => {
    const user = userEvent.setup();
    render(<App backend={createMockBackend()} />);
    await screen.findByRole('button', { name: /^readme\.txt/ });

    const menu = await openMenuFor(user, 'readme.txt');
    await user.click(within(menu).getByRole('menuitem', { name: 'Delete' }));
    await user.click(
      within(screen.getByRole('dialog')).getByRole('button', { name: 'Cancel' }),
    );

    expect(screen.getByRole('button', { name: /^readme\.txt/ })).toBeInTheDocument();
  });

  it('closes a dialog on Escape and gives focus back to what opened it', async () => {
    const user = userEvent.setup();
    render(<App backend={createMockBackend()} />);
    await screen.findByRole('button', { name: /^readme\.txt/ });

    const opener = screen.getByRole('button', { name: 'New folder' });
    await user.click(opener);
    expect(screen.getByRole('dialog')).toBeInTheDocument();

    await user.keyboard('{Escape}');
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(opener).toHaveFocus();
  });

  it('walks a dropped path into the folder that is open', async () => {
    const user = userEvent.setup();
    const mock = createMockBackend();
    render(<App backend={mock} />);

    await user.click(await screen.findByRole('button', { name: /^Photos/ }));
    await screen.findByRole('heading', { name: 'Photos' });

    mock.emitDrop(['/tmp/gull.jpg']);
    expect(await screen.findByRole('button', { name: /^gull\.jpg/ })).toBeInTheDocument();
    expect(mock.entries.has('Photos/gull.jpg')).toBe(true);
  });

  it('invites a first file when the folder is empty, and a device when none is paired', async () => {
    const mock = createMockBackend({ entries: [], state: { peers: [] } });
    render(<App backend={mock} />);

    expect(await screen.findByText(/Nothing here yet/)).toBeInTheDocument();
    expect(screen.getByText('Pair your other device')).toBeInTheDocument();
  });

  it('drops the pairing card once there is a device to sync with', async () => {
    const mock = createMockBackend({ entries: [] });
    render(<App backend={mock} />);

    expect(await screen.findByText(/Nothing here yet/)).toBeInTheDocument();
    expect(screen.queryByText('Pair your other device')).toBeNull();
  });

  it('says so when an action fails, rather than doing nothing visible', async () => {
    const user = userEvent.setup();
    const mock = createMockBackend();
    mock.pickAndImport = () => Promise.reject(new Error('no'));
    render(<App backend={mock} />);
    await screen.findByRole('button', { name: /^readme\.txt/ });

    await user.click(screen.getByRole('button', { name: 'Add files' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('Those files could not be added.');
  });
});

describe('the row menu', () => {
  it('walks its items with the arrow keys and closes on Escape', async () => {
    const user = userEvent.setup();
    render(<App backend={createMockBackend()} />);
    await screen.findByRole('button', { name: /^readme\.txt/ });

    const opener = screen.getByRole('button', { name: 'More for readme.txt' });
    await user.click(opener);
    expect(opener).toHaveAttribute('aria-expanded', 'true');
    expect(screen.getByRole('menuitem', { name: 'Open' })).toHaveFocus();

    await user.keyboard('{ArrowDown}');
    expect(screen.getByRole('menuitem', { name: 'Rename' })).toHaveFocus();
    await user.keyboard('{ArrowUp}{ArrowUp}');
    expect(screen.getByRole('menuitem', { name: 'Delete' })).toHaveFocus();

    await user.keyboard('{Escape}');
    expect(screen.queryByRole('menu')).toBeNull();
    expect(opener).toHaveFocus();
  });

  it('closes when the press lands anywhere else', async () => {
    const user = userEvent.setup();
    render(<App backend={createMockBackend()} />);
    await screen.findByRole('button', { name: /^readme\.txt/ });

    await user.click(screen.getByRole('button', { name: 'More for readme.txt' }));
    const menu = screen.getByRole('menu');

    // The layer over the window is what catches that press, so there is no
    // document listener to add and forget to remove.
    fireEvent.mouseDown(menu.parentElement!);
    expect(screen.queryByRole('menu')).toBeNull();
  });

  it('opens the entry from the menu, same as pressing the row', async () => {
    const user = userEvent.setup();
    render(<App backend={createMockBackend()} />);
    await screen.findByRole('button', { name: /^Photos/ });

    await user.click(screen.getByRole('button', { name: 'More for Photos' }));
    await user.click(screen.getByRole('menuitem', { name: 'Open' }));

    expect(await screen.findByRole('heading', { name: 'Photos' })).toBeInTheDocument();
  });
});
