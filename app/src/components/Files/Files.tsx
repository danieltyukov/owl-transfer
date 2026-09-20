import { useCallback, useEffect, useState } from 'react';

import type { Backend, DirEntry, State } from '../../backend/types.js';
import { PlusGlyph } from '../../icons/glyphs.js';
import { OwlMark } from '../../icons/OwlMark.js';
import type { Pane } from '../../panes.js';
import { Breadcrumb } from './Breadcrumb.js';
import { FileRow } from './FileRow.js';
import './Files.css';

export interface FilesProps {
  backend: Backend;
  state: State;
  /** Relative to the sync folder. The empty string is the folder itself. */
  dir: string;
  onDir: (dir: string) => void;
  onPane: (pane: Pane) => void;
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

/*
 * The folder.
 *
 * Everything on this screen comes from the backend: the listing on arrival, and
 * a fresh listing whenever the engine says this directory went stale. There is
 * no local model of the tree to keep in step, which is what makes a file that
 * lands from the other device simply appear.
 */
export function Files({ backend, state, dir, onDir, onPane }: FilesProps) {
  const [entries, setEntries] = useState<DirEntry[] | null>(null);
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

  const open = (entry: DirEntry): void => {
    if (entry.isDir) onDir(entry.path);
    else void backend.openEntry(entry.path);
  };

  return (
    <>
      <div className="files-head" data-window-drag="">
        <OwlMark size={20} label="Owl Transfer" className="files-brand" />
        <Breadcrumb dir={dir} root={folderName(state.folder)} onDir={onDir} />
        <div className="files-tools">
          <button
            type="button"
            className="button"
            onClick={() => void backend.pickAndImport(dir)}
          >
            <PlusGlyph />
            Add files
          </button>
        </div>
      </div>

      <div className="pane-body">
        <ul className="files-list">
          {(entries ?? []).map(entry => (
            <FileRow
              key={entry.path}
              entry={entry}
              now={now}
              onOpen={open}
              onMenu={() => undefined}
            />
          ))}
        </ul>
      </div>
    </>
  );
}
