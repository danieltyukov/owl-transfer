import { act, fireEvent, render, screen, within } from '@testing-library/react';
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

  it('follows a transfer from the state, which is the only thing that moves', async () => {
    // The listing carries the status it had when it was asked for, and the
    // engine only calls a directory stale once the file has landed. Without
    // the state feeding the badge, the ring would never turn.
    const backend = createMockBackend();
    render(<App backend={backend} />);
    await screen.findByRole('button', { name: /^readme\.txt/ });

    const listings = vi.spyOn(backend, 'listDir');
    const moving = (bytesDone: number) => ({
      active: [
        {
          path: 'readme.txt',
          peerId: 'peer',
          direction: 'download' as const,
          bytesDone,
          bytesTotal: 1_204,
        },
      ],
      queued: 0,
      bytesPerSec: 1_000,
    });

    act(() => {
      backend.emitState({ transfers: moving(602) });
    });
    expect(screen.getByRole('button', { name: /^readme\.txt/ })).toHaveAccessibleName(
      /Syncing, 50 percent/,
    );

    act(() => {
      backend.emitState({ transfers: moving(903) });
    });
    expect(screen.getByRole('button', { name: /^readme\.txt/ })).toHaveAccessibleName(
      /Syncing, 75 percent/,
    );
    expect(listings).not.toHaveBeenCalled();
  });

  it('leaves a conflict copy marked as one while something else is in flight', async () => {
    const backend = createMockBackend();
    const user = userEvent.setup();
    render(<App backend={backend} />);
    await screen.findByRole('button', { name: /^readme\.txt/ });
    await user.click(screen.getByRole('button', { name: /^Recordings/ }));
    await screen.findByRole('button', { name: /^field notes\.md/ });

    act(() => {
      backend.emitState({
        transfers: {
          active: [
            {
              path: 'Recordings/field notes.md',
              peerId: 'peer',
              direction: 'download',
              bytesDone: 1,
              bytesTotal: 2,
            },
          ],
          queued: 0,
          bytesPerSec: 1_000,
        },
      });
    });
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

  it('lights the drop zone from the shell, which is the only place a window hears it', async () => {
    // A desktop window never gets `dragenter`: the shell takes the drag off the
    // web layer to read the paths out of it. Without this the highlight and the
    // hint are dead in the one place they were drawn for.
    const mock = createMockBackend();
    const { container } = render(<App backend={mock} />);
    await screen.findByRole('button', { name: /^readme\.txt/ });

    const zone = container.querySelector('.dropzone')!;
    expect(zone).not.toHaveAttribute('data-over');

    act(() => mock.emitDragOver(true));
    expect(zone).toHaveAttribute('data-over');

    // A drop ends the hover as surely as a leave does, and the adapter reports
    // both as false.
    act(() => mock.emitDragOver(false));
    expect(zone).not.toHaveAttribute('data-over');
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

  it('keeps Tab inside itself, since the page behind it cannot be pressed', async () => {
    const user = userEvent.setup();
    render(<App backend={createMockBackend()} />);
    await screen.findByRole('button', { name: /^readme\.txt/ });

    await user.click(screen.getByRole('button', { name: 'More for readme.txt' }));
    expect(screen.getByRole('menuitem', { name: 'Open' })).toHaveFocus();

    await user.tab();
    expect(screen.getByRole('menuitem', { name: 'Rename' })).toHaveFocus();
    await user.tab();
    await user.tab();
    // Round, not out into a page that is still under a layer swallowing every
    // press.
    expect(screen.getByRole('menuitem', { name: 'Open' })).toHaveFocus();

    await user.tab({ shift: true });
    expect(screen.getByRole('menuitem', { name: 'Delete' })).toHaveFocus();
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

describe('listings that land out of order', () => {
  it('never renders one folder under another folder’s name', async () => {
    const user = userEvent.setup();
    const mock = createMockBackend();

    // Every listing is held open, so the order they finish in is the test's.
    const waiting: Array<{ path: string; settle: () => void }> = [];
    const real = mock.listDir.bind(mock);
    mock.listDir = path =>
      new Promise(resolve => {
        waiting.push({ path, settle: () => void real(path).then(resolve) });
      });

    render(<App backend={mock} />);

    // The listing the pane opened with.
    await vi.waitFor(() => expect(waiting).toHaveLength(1));
    waiting[0]!.settle();
    await screen.findByRole('button', { name: /^Photos/ });

    // A change to this directory starts a second listing of it.
    mock.emitDirChanged('');
    await vi.waitFor(() => expect(waiting).toHaveLength(2));

    // Walk into a subfolder while that one is still in flight.
    await user.click(screen.getByRole('button', { name: /^Photos/ }));
    await vi.waitFor(() => expect(waiting).toHaveLength(3));
    waiting[2]!.settle();
    expect(await screen.findByRole('button', { name: /^lighthouse\.jpg/ })).toBeInTheDocument();

    // Now the stale one comes back. It is the parent's contents and it must
    // not appear under the child's breadcrumb.
    waiting[1]!.settle();
    await vi.waitFor(() =>
      expect(screen.getByRole('heading', { name: 'Photos' })).toBeInTheDocument(),
    );
    expect(screen.queryByRole('button', { name: /^readme\.txt/ })).toBeNull();
    expect(screen.getByRole('button', { name: /^lighthouse\.jpg/ })).toBeInTheDocument();
  });
});

describe('a long press on a touch screen', () => {
  const rowFor = (name: string): HTMLElement =>
    screen.getByRole('button', { name: `More for ${name}` }).closest('li')!;

  const longPress = async (name: string) => {
    const row = rowFor(name);
    fireEvent.pointerDown(row, { pointerType: 'touch', clientX: 40, clientY: 40 });
    // The menu appears when the 500ms timer fires.
    const menu = await screen.findByRole('menu', { name });
    fireEvent.pointerUp(row);
    return menu;
  };

  it('opens the menu without also opening the entry', async () => {
    render(<App backend={createMockBackend()} />);
    await screen.findByRole('button', { name: /^Photos/ });

    await longPress('Photos');

    // The browser makes a click out of the same touch. Without the guard it
    // lands on the row and walks into the folder behind the open menu.
    fireEvent.click(screen.getByRole('button', { name: /^Photos/ }));
    expect(screen.getByRole('heading', { name: 'OwlTransfer' })).toBeInTheDocument();
    expect(screen.getByRole('menu', { name: 'Photos' })).toBeInTheDocument();
  });

  it('leaves the next press alone, so the row is not dead afterwards', async () => {
    const user = userEvent.setup();
    render(<App backend={createMockBackend()} />);
    await screen.findByRole('button', { name: /^Photos/ });

    await longPress('Photos');
    await user.keyboard('{Escape}');

    // One task after the finger lifts the guard is gone, and a plain press
    // opens the folder the way it always did.
    await user.click(screen.getByRole('button', { name: /^Photos/ }));
    expect(await screen.findByRole('heading', { name: 'Photos' })).toBeInTheDocument();
  });

  it('does not arm on a scroll that happens to start on a row', async () => {
    render(<App backend={createMockBackend()} />);
    await screen.findByRole('button', { name: /^Photos/ });

    const row = rowFor('Photos');
    fireEvent.pointerDown(row, { pointerType: 'touch', clientX: 40, clientY: 40 });
    fireEvent.pointerMove(row, { pointerType: 'touch', clientX: 40, clientY: 90 });
    fireEvent.pointerUp(row);

    await new Promise(resolve => setTimeout(resolve, 600));
    expect(screen.queryByRole('menu')).toBeNull();
  });
});
