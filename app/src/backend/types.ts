/*
 * The one seam between the interface and everything underneath it.
 *
 * The React app knows this file and nothing else: no Tauri import, no command
 * name, no event name. The shell implements `Backend` and hands it over; a
 * browser gets the mock. That is what lets the whole interface be developed and
 * tested without an engine, and it is what keeps the engine free to change its
 * command names without touching a component.
 *
 * `State` is the whole picture in one object, pushed by the engine on every
 * change. Nothing in the interface keeps sync state of its own, so there is no
 * second copy to fall behind.
 */

export type DeviceKind = 'desktop' | 'phone';

export interface DeviceInfo {
  id: string;
  name: string;
  kind: DeviceKind;
  port: number;
  /**
   * This machine's non-loopback IPv4 addresses, without a port. The interface
   * shows each one with the port beside it, so a person can read an address off
   * this device and type it into the other when discovery cannot work. Empty
   * while the machine is on no network.
   */
  addresses: string[];
}

export interface PeerInfo {
  id: string;
  name: string;
  kind: DeviceKind;
  connected: boolean;
  address: string | null;
  lastSeenMs: number | null;
  pairedAtMs: number;
}

export interface NearbyInfo {
  id: string;
  name: string;
  kind: DeviceKind;
  address: string;
}

export interface PairingInfo {
  id: string;
  name: string;
  kind: DeviceKind;
  code: string;
  direction: 'incoming' | 'outgoing';
}

export interface Transfer {
  path: string;
  peerId: string;
  direction: 'download' | 'upload';
  bytesDone: number;
  bytesTotal: number;
}

export interface TransferSummary {
  active: Transfer[];
  queued: number;
  bytesPerSec: number;
}

export interface SyncSummary {
  files: number;
  dirs: number;
  bytes: number;
  lastChangeMs: number | null;
  upToDate: boolean;
}

export interface State {
  device: DeviceInfo;
  folder: string;
  paused: boolean;
  peers: PeerInfo[];
  nearby: NearbyInfo[];
  pendingPairing: PairingInfo | null;
  transfers: TransferSummary;
  summary: SyncSummary;
  errors: string[];
}

export type EntryStatus =
  | { kind: 'synced' }
  | { kind: 'syncing'; progress: number }
  | { kind: 'waiting' }
  | { kind: 'conflict' }
  | { kind: 'local' };

export interface DirEntry {
  name: string;
  path: string;
  isDir: boolean;
  size: number;
  mtimeMs: number;
  status: EntryStatus;
}

export type Unsubscribe = () => void;

export type Platform = 'desktop' | 'android' | 'browser';

export type Permission = 'granted' | 'denied' | 'not-applicable';

/** The window the shell drew no native bar for. Null everywhere else. */
export interface WindowFrame {
  minimize(): Promise<void>;
  toggleMaximize(): Promise<void>;
  close(): Promise<void>;
  startDrag(): Promise<void>;
  /**
   * Asked once when the controls mount. A window restored from a maximised
   * session is already maximised before anything here has run, and without
   * this the control would offer to maximise what already is.
   */
  isMaximized(): Promise<boolean>;
  /**
   * Followed afterwards. The window manager maximises a window by other routes
   * than this app's own button: a double press on the title bar, a keyboard
   * shortcut, a drag to the top of the screen.
   */
  onMaximizedChange(cb: (maximized: boolean) => void): Unsubscribe;
}

export interface Backend {
  readonly platform: Platform;
  readonly version: string;
  readonly window: WindowFrame | null;
  getState(): Promise<State>;
  onState(cb: (s: State) => void): Unsubscribe;
  onDirChanged(cb: (path: string) => void): Unsubscribe;
  onDrop(cb: (paths: string[]) => void): Unsubscribe;
  /**
   * Whether something draggable is over the window.
   *
   * The interface cannot answer this for itself where there is a shell. A
   * desktop window has the drag intercepted before the web layer sees it, so
   * that the paths can be read off it, and `dragenter` never fires. A browser
   * has no shell to intercept anything and keeps its own events, so there this
   * never fires and the zone lights up from those instead.
   */
  onDragOver(cb: (over: boolean) => void): Unsubscribe;
  listDir(path: string): Promise<DirEntry[]>;
  pickAndImport(into: string): Promise<number>;
  importPaths(paths: string[], into: string): Promise<number>;
  createFolder(path: string): Promise<void>;
  deleteEntry(path: string): Promise<void>;
  renameEntry(path: string, newName: string): Promise<void>;
  openEntry(path: string): Promise<void>;
  revealFolder(): Promise<void>;
  pickFolder(): Promise<string | null>;
  setFolder(path: string): Promise<void>;
  setDeviceName(name: string): Promise<void>;
  pairWithNearby(id: string): Promise<void>;
  pairWithAddress(host: string, port: number): Promise<void>;
  respondToPairing(id: string, accept: boolean): Promise<void>;
  forgetPeer(id: string): Promise<void>;
  allFilesPermission(): Promise<Permission>;
  openAllFilesSettings(): Promise<void>;
  /**
   * Stops or restarts watching, scanning and syncing.
   *
   * Android starts paused, because the sync folder is in shared storage and is
   * out of reach until all files access is granted. The permission card calls
   * this with `false` the moment it sees the grant, which is the only caller:
   * there is no pause control, because a folder that silently stops syncing is
   * not a feature.
   */
  setPaused(paused: boolean): Promise<void>;
  openUrl(url: string): Promise<void>;
}
