import { describe, expect, it, vi } from 'vitest';

import { MOCK_PAIRING_CODE, createMockBackend } from './mock.js';

describe('the mock backend', () => {
  it('lists only the entries directly inside a directory', async () => {
    const mock = createMockBackend();
    const root = await mock.listDir('');
    expect(root.map(e => e.name).sort()).toEqual([
      'Photos',
      'Recordings',
      'kestrel.png',
      'owl calls.zip',
      'readme.txt',
    ]);
    expect((await mock.listDir('Photos')).map(e => e.name)).toEqual([
      'harbour at dusk.jpg',
      'lighthouse.jpg',
    ]);
  });

  it('creates a folder and announces the directory that went stale', async () => {
    const mock = createMockBackend();
    const changed = vi.fn();
    mock.onDirChanged(changed);

    await mock.createFolder('Scans');
    expect((await mock.listDir('')).map(e => e.name)).toContain('Scans');
    expect(changed).toHaveBeenCalledWith('');
  });

  it('deletes a folder together with everything under it', async () => {
    const mock = createMockBackend();
    await mock.deleteEntry('Photos');
    expect((await mock.listDir('')).map(e => e.name)).not.toContain('Photos');
    expect(await mock.listDir('Photos')).toEqual([]);
  });

  it('carries the children along when a folder is renamed', async () => {
    const mock = createMockBackend();
    await mock.renameEntry('Photos', 'Pictures');
    expect((await mock.listDir('Pictures')).map(e => e.path)).toEqual([
      'Pictures/harbour at dusk.jpg',
      'Pictures/lighthouse.jpg',
    ]);
  });

  it('gives an imported file a free name rather than overwriting one', async () => {
    const mock = createMockBackend();
    await mock.importPaths(['/tmp/readme.txt'], '');
    const names = (await mock.listDir('')).map(e => e.name);
    expect(names).toContain('readme.txt');
    expect(names).toContain('readme 2.txt');
  });

  it('shows the six digits both devices compare, then promotes on accept', async () => {
    const mock = createMockBackend();
    const nearby = mock.state.nearby[0]!;

    await mock.pairWithNearby(nearby.id);
    expect(mock.state.pendingPairing).toMatchObject({
      id: nearby.id,
      code: MOCK_PAIRING_CODE,
      direction: 'outgoing',
    });

    await mock.respondToPairing(nearby.id, true);
    expect(mock.state.pendingPairing).toBeNull();
    expect(mock.state.peers.map(p => p.id)).toContain(nearby.id);
    expect(mock.state.nearby).toEqual([]);
  });

  it('leaves the device where it was when the request is declined', async () => {
    const mock = createMockBackend();
    const nearby = mock.state.nearby[0]!;
    await mock.pairWithNearby(nearby.id);
    await mock.respondToPairing(nearby.id, false);

    expect(mock.state.pendingPairing).toBeNull();
    expect(mock.state.peers.map(p => p.id)).not.toContain(nearby.id);
    expect(mock.state.nearby.map(n => n.id)).toContain(nearby.id);
  });

  it('pushes every state change to whoever is watching, until they stop', async () => {
    const mock = createMockBackend();
    const seen = vi.fn();
    const off = mock.onState(seen);

    await mock.setDeviceName('attic');
    expect(seen).toHaveBeenLastCalledWith(expect.objectContaining({ device: expect.objectContaining({ name: 'attic' }) }));

    off();
    await mock.forgetPeer(mock.state.peers[0]!.id);
    expect(seen).toHaveBeenCalledTimes(1);
  });

  it('takes a seed, so a test can put the app on a phone with no permission', async () => {
    const mock = createMockBackend({ platform: 'android', permission: 'denied' });
    expect(mock.platform).toBe('android');
    expect(await mock.allFilesPermission()).toBe('denied');

    await mock.openAllFilesSettings();
    expect(await mock.allFilesPermission()).toBe('granted');
  });
});
