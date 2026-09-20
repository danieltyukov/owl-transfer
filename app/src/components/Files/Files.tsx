import { useCallback, useEffect, useState } from 'react';

import type { Backend, DirEntry, State } from '../../backend/types.js';
import { FolderPlusGlyph, PlusGlyph } from '../../icons/glyphs.js';
import { OwlMark } from '../../icons/OwlMark.js';
import type { Pane } from '../../panes.js';
import { Dialog } from '../Dialog.js';
import { Breadcrumb } from './Breadcrumb.js';
import { DropZone } from './DropZone.js';
import { EmptyState } from './EmptyState.js';
import { EntryMenu } from './EntryMenu.js';
import { FileRow } from './FileRow.js';
import './Files.css';

export interface FilesProps {
  backend: Backend;
  state: State;
  /** Relative to the sync folder. The empty string is the folder itself. */
  dir: string;
  onDir: (dir: string) => void;
  onPane: (pane: Pane) => void;
  onError: (message: string) => void;
}

/** Folders first, then names the way a person reads them: "file 10" after "file 9". */
const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: 'base' });

export function sortEntries(entries: readonly DirEntry[]): DirEntry[] {
  return [...entries].sort((a, b) => {
    if (a.isDir !== b.isDir) return a.isDir ? -1 : 1;
    return collator.compare(a.name, b.name);
  });
}

/** The last segment of the sync folder's own path, which heads the breadcrumb. */
export function folderName(path: string): string {
  const parts = path.split(/[/\\]/).filter(part => part !== '');
  return parts[parts.length - 1] ?? path;
}

/** A name a file can actually have, on every platform the app runs on. */
export function nameProblem(name: string): string | null {
  const trimmed = name.trim();
  if (trimmed === '') return 'Give it a name.';
  if (trimmed === '.' || trimmed === '..') return 'That name is reserved.';
  if (/[/\\]/.test(trimmed)) return 'A name cannot contain a slash.';
  return null;
}

type Dialogue =
  | { kind: 'new-folder' }
  | { kind: 'rename'; entry: DirEntry }
  | { kind: 'delete'; entry: DirEntry };

/*
 * The folder.
 *
 * Everything on this screen comes from the backend: the listing on arrival, and
 * a fresh listing whenever the engine says this directory went stale. There is
 * no local model of the tree to keep in step, which is what makes a file that
 * lands from the other device simply appear.
 */
export function Files({ backend, state, dir, onDir, onPane, onError }: FilesProps) {
  const [entries, setEntries] = useState<DirEntry[] | null>(null);
  const [menu, setMenu] = useState<{ entry: DirEntry; at: { x: number; y: number } } | null>(null);
  const [dialogue, setDialogue] = useState<Dialogue | null>(null);
  const [draft, setDraft] = useState('');
  const now = Date.now();

  const reload = useCallback(() => {
    let live = true;
    void backend.listDir(dir).then(
      next => {
        if (live) setEntries(sortEntries(next));
      },
      () => {
        if (live) setEntries([]);
      },
    );
    return () => {
      live = false;
    };
  }, [backend, dir]);

  useEffect(() => reload(), [reload]);

  useEffect(
    () =>
      backend.onDirChanged(changed => {
        // A change to this directory, or to one it sits under: the second can
        // mean this directory no longer exists.
        if (changed === '' || changed === dir || dir.startsWith(`${changed}/`)) reload();
      }),
    [backend, dir, reload],
  );

  // A real drop comes from the shell with paths on disk. The browser's own drop
  // event carries File objects the engine cannot open, so it only lights up.
  useEffect(
    () =>
      backend.onDrop(paths => {
        void backend.importPaths(paths, dir).catch(() => onError('Those files could not be added.'));
      }),
    [backend, dir, onError],
  );

  const open = (entry: DirEntry): void => {
    if (entry.isDir) onDir(entry.path);
    else void backend.openEntry(entry.path).catch(() => onError(`${entry.name} could not be opened.`));
  };

  const addFiles = (): void => {
    void backend.pickAndImport(dir).catch(() => onError('Those files could not be added.'));
  };

  const startDialogue = (next: Dialogue): void => {
    setMenu(null);
    setDraft(next.kind === 'rename' ? next.entry.name : '');
    setDialogue(next);
  };

  const problem = nameProblem(draft);

  const submit = (): void => {
    const open = dialogue;
    setDialogue(null);
    if (open === null) return;
    const name = draft.trim();

    if (open.kind === 'new-folder') {
      void backend
        .createFolder(dir === '' ? name : `${dir}/${name}`)
        .catch(() => onError(`${name} could not be created.`));
    } else if (open.kind === 'rename') {
      void backend
        .renameEntry(open.entry.path, name)
        .catch(() => onError(`${open.entry.name} could not be renamed.`));
    } else {
      void backend
        .deleteEntry(open.entry.path)
        .catch(() => onError(`${open.entry.name} could not be deleted.`));
    }
  };

  const listing = entries ?? [];
  const here = dir === '' ? folderName(state.folder) : dir.slice(dir.lastIndexOf('/') + 1);

  return (
    <>
      <div className="files-head" data-window-drag="">
        <OwlMark size={20} label="Owl Transfer" className="files-brand" />
        <Breadcrumb dir={dir} root={folderName(state.folder)} onDir={onDir} />
        <div className="files-tools">
          <button type="button" className="button" onClick={addFiles}>
            <PlusGlyph />
            Add files
          </button>
          <button
            type="button"
            className="button"
            onClick={() => startDialogue({ kind: 'new-folder' })}
          >
            <FolderPlusGlyph />
            New folder
          </button>
        </div>
      </div>

      <div className="pane-body">
        <DropZone label={here}>
          {entries === null ? null : listing.length === 0 ? (
            <EmptyState
              root={dir === ''}
              hasPeers={state.peers.length > 0}
              onAdd={addFiles}
              onNewFolder={() => startDialogue({ kind: 'new-folder' })}
              onPairDevice={() => onPane('devices')}
            />
          ) : (
            <ul className="files-list">
              {listing.map(entry => (
                <FileRow
                  key={entry.path}
                  entry={entry}
                  now={now}
                  onOpen={open}
                  onMenu={(target, at) => setMenu({ entry: target, at })}
                />
              ))}
            </ul>
          )}
        </DropZone>
      </div>

      {menu === null ? null : (
        <EntryMenu
          entry={menu.entry}
          at={menu.at}
          onOpen={() => {
            const entry = menu.entry;
            setMenu(null);
            open(entry);
          }}
          onRename={() => startDialogue({ kind: 'rename', entry: menu.entry })}
          onDelete={() => startDialogue({ kind: 'delete', entry: menu.entry })}
          onClose={() => setMenu(null)}
        />
      )}

      {dialogue === null ? null : dialogue.kind === 'delete' ? (
        <Dialog
          title={`Delete ${dialogue.entry.name}?`}
          description={
            dialogue.entry.isDir
              ? 'The folder and everything in it goes from both devices.'
              : 'It goes from both devices.'
          }
          submitLabel="Delete"
          danger
          onSubmit={submit}
          onClose={() => setDialogue(null)}
        />
      ) : (
        <Dialog
          title={dialogue.kind === 'rename' ? `Rename ${dialogue.entry.name}` : 'New folder'}
          submitLabel={dialogue.kind === 'rename' ? 'Rename' : 'Create'}
          submitDisabled={problem !== null}
          onSubmit={submit}
          onClose={() => setDialogue(null)}
        >
          <input
            className="input dialog-field"
            aria-label="Name"
            aria-invalid={draft !== '' && problem !== null ? true : undefined}
            value={draft}
            placeholder={dialogue.kind === 'rename' ? undefined : 'Photos'}
            onChange={event => setDraft(event.target.value)}
          />
          {draft !== '' && problem !== null ? <p className="dialog-text">{problem}</p> : null}
        </Dialog>
      )}
    </>
  );
}
