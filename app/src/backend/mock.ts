import type {
  Backend,
  DirEntry,
  EntryStatus,
  NearbyInfo,
  PeerInfo,
  Permission,
  Platform,
  State,
  Unsubscribe,
  WindowFrame,
} from './types.js';

/*
 * The backend the interface runs on when there is no engine underneath it: in a
 * browser during development, and in every component test.
 *
 * It is a real implementation of the contract, not a pile of stubs. The tree is
 * a map of paths, every mutating call changes it and announces the directory
 * that went stale, and pairing walks the same three states the engine does. A
 * mock that only returned fixtures would let the interface drift into asking
 * for things the engine cannot do, which is the failure this exists to prevent.
 */

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;

/** The six digits both devices show while pairing. Fixed, so a test can read it. */
export const MOCK_PAIRING_CODE = '482 913';

export interface MockSeed {
  platform?: Platform;
  version?: string;
  window?: WindowFrame | null;
  permission?: Permission;
  /** Wall clock the tree's modification times are measured back from. */
  now?: number;
  state?: Partial<State>;
  /** Replaces the default tree entirely. */
  entries?: DirEntry[];
}

export interface MockFrame extends WindowFrame {
  /** Maximise or restore the window the way a window manager would. */
  setMaximized(maximized: boolean): void;
  readonly maximized: boolean;
}

/*
 * A window frame with no window behind it.
 *
 * It keeps the one piece of state the real frame has, so the controls can be
 * developed and tested without a shell: `toggleMaximize` flips it and tells
 * whoever is watching, and `setMaximized` is the other route in, standing for
 * the double press on the title bar and the window manager's own shortcuts.
 */
export function createMockFrame(maximized = false): MockFrame {
  let state = maximized;
  const listeners = new Set<(maximized: boolean) => void>();

  const announce = (): void => {
    for (const cb of [...listeners]) cb(state);
  };

  return {
    get maximized() {
      return state;
    },
    setMaximized(next) {
      if (next === state) return;
      state = next;
      announce();
    },
    minimize: () => Promise.resolve(),
    toggleMaximize() {
      state = !state;
      announce();
      return Promise.resolve();
    },
    close: () => Promise.resolve(),
    startDrag: () => Promise.resolve(),
    isMaximized: () => Promise.resolve(state),
    onMaximizedChange(cb): Unsubscribe {
      listeners.add(cb);
      return () => {
        listeners.delete(cb);
      };
    },
  };
}

export interface MockBackend extends Backend {
  /** Merge a patch into the state and push it, the way the engine would. */
  emitState(patch: Partial<State>): void;
  emitDirChanged(path: string): void;
  emitDrop(paths: string[]): void;
  readonly state: State;
  readonly entries: ReadonlyMap<string, DirEntry>;
}

const parentOf = (path: string): string => {
  const at = path.lastIndexOf('/');
  return at === -1 ? '' : path.slice(0, at);
};

const join = (dir: string, name: string): string => (dir === '' ? name : `${dir}/${name}`);

const synced: EntryStatus = { kind: 'synced' };

function defaultEntries(now: number): DirEntry[] {
  const file = (
    path: string,
    size: number,
    ago: number,
    status: EntryStatus = synced,
  ): DirEntry => ({
    name: path.slice(path.lastIndexOf('/') + 1),
    path,
    isDir: false,
    size,
    mtimeMs: now - ago,
    status,
  });
  const dir = (path: string, ago: number): DirEntry => ({
    name: path.slice(path.lastIndexOf('/') + 1),
    path,
    isDir: true,
    size: 0,
    mtimeMs: now - ago,
    status: synced,
  });

  return [
    // Deliberately out of order: the list component sorts, and a fixture that
    // arrives sorted would never prove it.
    file('readme.txt', 1_204, 3 * DAY),
    dir('Photos', 12 * MINUTE),
    file('kestrel.png', 3_140_000, 40_000, { kind: 'syncing', progress: 0.42 }),
    dir('Recordings', 2 * HOUR),
    file('owl calls.zip', 48_200_000, 5 * HOUR, { kind: 'local' }),
    file('Photos/harbour at dusk.jpg', 2_430_000, 12 * MINUTE),
    file('Photos/lighthouse.jpg', 1_870_000, 26 * HOUR),
    file('Recordings/barn owl 4am.wav', 18_600_000, 2 * HOUR, { kind: 'waiting' }),
    file('Recordings/field notes.md', 2_940, 3 * HOUR, { kind: 'conflict' }),
  ];
}

function defaultState(now: number): State {
  const phone: PeerInfo = {
    id: '9f2c71aa4d38e5b0',
    name: 'Pixel',
    kind: 'phone',
    connected: true,
    address: '192.168.1.24:52734',
    lastSeenMs: now,
    pairedAtMs: now - 6 * DAY,
  };
  const studio: NearbyInfo = {
    id: '4d7a0c93be115f62',
    name: 'studio',
    kind: 'desktop',
    address: '192.168.1.31:52734',
  };
  return {
    device: {
      id: 'c081e4f7a2d6b93c',
      name: 'workshop',
      kind: 'desktop',
      port: 52734,
      // Two, because a machine on a wired network and a VPN has two, and the
      // card has to say which is which by showing both.
      addresses: ['192.168.1.20', '10.0.0.5'],
    },
    folder: '/home/you/OwlTransfer',
    paused: false,
    peers: [phone],
    nearby: [studio],
    pendingPairing: null,
    transfers: {
      active: [
        {
          path: 'kestrel.png',
          peerId: phone.id,
          direction: 'download',
          bytesDone: 1_318_800,
          bytesTotal: 3_140_000,
        },
      ],
      queued: 1,
      bytesPerSec: 12_400_000,
    },
    summary: {
      files: 6,
      dirs: 2,
      bytes: 76_087_144,
      lastChangeMs: now - 40_000,
      upToDate: false,
    },
    errors: [],
  };
}

export function createMockBackend(seed: MockSeed = {}): MockBackend {
  const now = seed.now ?? Date.now();
  const tree = new Map<string, DirEntry>();
  for (const entry of seed.entries ?? defaultEntries(now)) tree.set(entry.path, entry);

  let state: State = { ...defaultState(now), ...seed.state };
  let permission: Permission = seed.permission ?? 'not-applicable';
  let picked = 0;

  const stateListeners = new Set<(s: State) => void>();
  const dirListeners = new Set<(path: string) => void>();
  const dropListeners = new Set<(paths: string[]) => void>();

  const push = (): void => {
    for (const cb of [...stateListeners]) cb(state);
  };
  const announce = (path: string): void => {
    for (const cb of [...dirListeners]) cb(path);
  };

  const put = (entry: DirEntry): void => {
    tree.set(entry.path, entry);
    announce(parentOf(entry.path));
  };

  /** A name that is not taken in `dir`, so a second import does not overwrite. */
  const freeName = (dir: string, name: string): string => {
    if (!tree.has(join(dir, name))) return name;
    const dot = name.lastIndexOf('.');
    const [stem, ext] = dot > 0 ? [name.slice(0, dot), name.slice(dot)] : [name, ''];
    for (let n = 2; ; n++) {
      const candidate = `${stem} ${n}${ext}`;
      if (!tree.has(join(dir, candidate))) return candidate;
    }
  };

  const addFile = (dir: string, name: string, size: number): void => {
    const free = freeName(dir, name);
    put({
      name: free,
      path: join(dir, free),
      isDir: false,
      size,
      mtimeMs: Date.now(),
      status: { kind: 'waiting' },
    });
  };

  const promote = (id: string): void => {
    const nearby = state.nearby.find(n => n.id === id);
    const pending = state.pendingPairing;
    const name = nearby?.name ?? pending?.name ?? id.slice(0, 8);
    const kind = nearby?.kind ?? pending?.kind ?? 'desktop';
    state = {
      ...state,
      peers: [
        ...state.peers,
        {
          id,
          name,
          kind,
          connected: true,
          address: nearby?.address ?? null,
          lastSeenMs: Date.now(),
          pairedAtMs: Date.now(),
        },
      ],
      nearby: state.nearby.filter(n => n.id !== id),
    };
  };

  const backend: MockBackend = {
    platform: seed.platform ?? 'browser',
    version: seed.version ?? '0.1.0',
    window: seed.window ?? null,

    get state() {
      return state;
    },
    get entries() {
      return tree;
    },

    emitState(patch) {
      state = { ...state, ...patch };
      push();
    },
    emitDirChanged(path) {
      announce(path);
    },
    emitDrop(paths) {
      for (const cb of [...dropListeners]) cb(paths);
    },

    getState: () => Promise.resolve(state),

    onState(cb): Unsubscribe {
      stateListeners.add(cb);
      return () => {
        stateListeners.delete(cb);
      };
    },
    onDirChanged(cb): Unsubscribe {
      dirListeners.add(cb);
      return () => {
        dirListeners.delete(cb);
      };
    },
    onDrop(cb): Unsubscribe {
      dropListeners.add(cb);
      return () => {
        dropListeners.delete(cb);
      };
    },

    listDir: path =>
      Promise.resolve([...tree.values()].filter(entry => parentOf(entry.path) === path)),

    pickAndImport(into) {
      picked += 1;
      addFile(into, `picked ${picked}.txt`, 4_096 * picked);
      return Promise.resolve(1);
    },

    importPaths(paths, into) {
      for (const [index, path] of paths.entries()) {
        const name = path.split(/[/\\]/).pop() ?? `dropped ${index + 1}`;
        addFile(into, name, 1_048_576 * (index + 1));
      }
      return Promise.resolve(paths.length);
    },

    createFolder(path) {
      put({
        name: path.slice(path.lastIndexOf('/') + 1),
        path,
        isDir: true,
        size: 0,
        mtimeMs: Date.now(),
        status: { kind: 'synced' },
      });
      return Promise.resolve();
    },

    deleteEntry(path) {
      // A directory takes everything under it, which is what the engine's
      // tombstone sweep does and what the confirmation warns about.
      for (const key of [...tree.keys()]) {
        if (key === path || key.startsWith(`${path}/`)) tree.delete(key);
      }
      announce(parentOf(path));
      return Promise.resolve();
    },

    renameEntry(path, newName) {
      const entry = tree.get(path);
      if (entry === undefined) return Promise.reject(new Error(`no entry at ${path}`));
      const dir = parentOf(path);
      const target = join(dir, newName);
      for (const [key, value] of [...tree.entries()]) {
        if (key !== path && !key.startsWith(`${path}/`)) continue;
        tree.delete(key);
        const moved = key === path ? target : target + key.slice(path.length);
        tree.set(moved, {
          ...value,
          name: moved.slice(moved.lastIndexOf('/') + 1),
          path: moved,
        });
      }
      announce(dir);
      return Promise.resolve();
    },

    openEntry: () => Promise.resolve(),
    revealFolder: () => Promise.resolve(),
    pickFolder: () => Promise.resolve('/home/you/Shared'),

    setFolder(path) {
      state = { ...state, folder: path };
      push();
      return Promise.resolve();
    },

    setDeviceName(name) {
      state = { ...state, device: { ...state.device, name } };
      push();
      return Promise.resolve();
    },

    pairWithNearby(id) {
      const nearby = state.nearby.find(n => n.id === id);
      if (nearby === undefined) return Promise.reject(new Error(`no device ${id} nearby`));
      state = {
        ...state,
        pendingPairing: {
          id: nearby.id,
          name: nearby.name,
          kind: nearby.kind,
          code: MOCK_PAIRING_CODE,
          direction: 'outgoing',
        },
      };
      push();
      return Promise.resolve();
    },

    pairWithAddress(host, port) {
      state = {
        ...state,
        pendingPairing: {
          id: `${host}:${port}`,
          name: `${host}:${port}`,
          kind: 'desktop',
          code: MOCK_PAIRING_CODE,
          direction: 'outgoing',
        },
      };
      push();
      return Promise.resolve();
    },

    respondToPairing(id, accept) {
      if (accept) promote(id);
      state = { ...state, pendingPairing: null };
      push();
      return Promise.resolve();
    },

    forgetPeer(id) {
      state = { ...state, peers: state.peers.filter(p => p.id !== id) };
      push();
      return Promise.resolve();
    },

    allFilesPermission: () => Promise.resolve(permission),

    openAllFilesSettings() {
      // The system screen is where the grant happens, so the app only ever
      // sees the result. Granting on return is what the real one looks like.
      permission = 'granted';
      return Promise.resolve();
    },

    openUrl: () => Promise.resolve(),
  };

  return backend;
}
