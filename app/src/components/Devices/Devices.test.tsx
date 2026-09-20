import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import { App } from '../../App.js';
import { spellCode } from './PairingCard.js';
import { createMockBackend, type MockBackend } from '../../backend/mock.js';

const openDevices = async (user: ReturnType<typeof userEvent.setup>) => {
  const sidebar = await screen.findByRole('navigation', { name: 'Main' });
  await user.click(within(sidebar).getByRole('button', { name: 'Devices' }));
  return screen.findByRole('heading', { name: 'Devices', level: 1 });
};

const show = async (mock: MockBackend) => {
  const user = userEvent.setup();
  render(<App backend={mock} />);
  await openDevices(user);
  return user;
};

describe('the devices pane', () => {
  it('lists what is paired and what is merely in earshot', async () => {
    await show(createMockBackend());

    const paired = screen.getByRole('region', { name: 'Paired' });
    expect(within(paired).getByText('Pixel')).toBeInTheDocument();
    expect(within(paired).getByText('192.168.1.24:52734')).toBeInTheDocument();

    const nearby = screen.getByRole('region', { name: 'Nearby' });
    expect(within(nearby).getByText('studio')).toBeInTheDocument();
  });

  it('shows when a peer was last heard from once it stops answering', async () => {
    const mock = createMockBackend();
    await show(mock);

    const peer = mock.state.peers[0]!;
    mock.emitState({
      peers: [{ ...peer, connected: false, lastSeenMs: Date.now() - 3 * 60_000 }],
    });
    expect(await screen.findByText('Last seen 3 min ago')).toBeInTheDocument();
  });

  it('puts the six digits up when pairing starts from the nearby list', async () => {
    const mock = createMockBackend();
    const user = await show(mock);

    const nearby = screen.getByRole('region', { name: 'Nearby' });
    await user.click(within(nearby).getByRole('button', { name: 'Pair' }));

    const card = await screen.findByRole('region', { name: 'Pairing' });
    expect(card).toHaveTextContent('Pairing with studio');
    expect(card).toHaveTextContent('482 913');
    expect(within(card).getByRole('button', { name: 'Cancel' })).toBeInTheDocument();
  });

  it('gives the code a name that is read back as six digits', async () => {
    const mock = createMockBackend();
    const user = await show(mock);

    await user.click(
      within(screen.getByRole('region', { name: 'Nearby' })).getByRole('button', { name: 'Pair' }),
    );
    const card = await screen.findByRole('region', { name: 'Pairing' });

    // Left alone, "482 913" is announced as "four hundred eighty-two, nine
    // hundred thirteen", which is not something anyone can check against
    // another screen.
    expect(within(card).getByLabelText('4 8 2, 9 1 3')).toHaveTextContent('482 913');
  });

  it('takes the request back when the outgoing card is cancelled', async () => {
    const mock = createMockBackend();
    const user = await show(mock);

    await user.click(
      within(screen.getByRole('region', { name: 'Nearby' })).getByRole('button', { name: 'Pair' }),
    );
    await user.click(
      within(await screen.findByRole('region', { name: 'Pairing' })).getByRole('button', {
        name: 'Cancel',
      }),
    );

    await vi.waitFor(() => expect(screen.queryByRole('region', { name: 'Pairing' })).toBeNull());
    expect(mock.state.peers.map(p => p.name)).not.toContain('studio');
  });

  it('accepts an incoming request and moves the device into Paired', async () => {
    const mock = createMockBackend();
    const user = await show(mock);
    const studio = mock.state.nearby[0]!;

    mock.emitState({
      pendingPairing: {
        id: studio.id,
        name: studio.name,
        kind: studio.kind,
        code: '482 913',
        direction: 'incoming',
      },
    });

    const card = await screen.findByRole('region', { name: 'Pairing' });
    expect(card).toHaveTextContent('studio wants to pair');
    await user.click(within(card).getByRole('button', { name: 'Accept' }));

    const paired = await screen.findByRole('region', { name: 'Paired' });
    expect(within(paired).getByText('studio')).toBeInTheDocument();
  });

  it('asks before forgetting a peer, then removes it', async () => {
    const mock = createMockBackend();
    const user = await show(mock);

    await user.click(
      within(screen.getByRole('region', { name: 'Paired' })).getByRole('button', {
        name: 'Forget',
      }),
    );
    const dialog = screen.getByRole('dialog', { name: 'Forget Pixel?' });
    expect(dialog).toHaveTextContent('Nothing already in the folder is deleted');

    await user.click(within(dialog).getByRole('button', { name: 'Forget' }));
    await vi.waitFor(() => expect(mock.state.peers).toEqual([]));
    expect(await screen.findByText(/Nothing is paired yet/)).toBeInTheDocument();
  });

  it('dials a typed address, which is the way round a network that blocks broadcasts', async () => {
    const mock = createMockBackend();
    const dial = vi.spyOn(mock, 'pairWithAddress');
    const user = await show(mock);

    await user.type(screen.getByLabelText('Address'), '10.0.2.2');
    await user.click(screen.getByRole('button', { name: 'Connect' }));

    expect(dial).toHaveBeenCalledWith('10.0.2.2', 52734);
  });

  it('will not dial an empty address or an impossible port', async () => {
    const user = await show(createMockBackend());
    const connect = screen.getByRole('button', { name: 'Connect' });
    expect(connect).toBeDisabled();

    await user.type(screen.getByLabelText('Address'), '10.0.2.2');
    expect(connect).toBeEnabled();

    await user.clear(screen.getByLabelText('Port'));
    await user.type(screen.getByLabelText('Port'), '99999');
    expect(connect).toBeDisabled();
  });

  it('shows what the other side has to type in, port and all', async () => {
    const mock = createMockBackend();
    await show(mock);

    const self = screen.getByRole('region', { name: 'This device' });
    expect(within(self).getByText('workshop')).toBeInTheDocument();
    expect(within(self).getByText('c081e4f7')).toBeInTheDocument();
    expect(within(self).getByText('192.168.1.20:52734')).toBeInTheDocument();
    expect(within(self).getByText('10.0.0.5:52734')).toBeInTheDocument();
    expect(self).toHaveTextContent('Type one of these on the other device to pair by address.');
  });

  it('names the row for however many addresses this machine has', async () => {
    const mock = createMockBackend();
    await show(mock);

    const self = screen.getByRole('region', { name: 'This device' });
    expect(within(self).getByText('Addresses')).toBeInTheDocument();

    mock.emitState({ device: { ...mock.state.device, addresses: ['192.168.1.20'] } });
    expect(await within(self).findByText('Address')).toBeInTheDocument();
    expect(within(self).queryByText('10.0.0.5:52734')).toBeNull();
  });

  it('falls back to the port alone while the machine is on no network', async () => {
    const mock = createMockBackend({
      state: { device: { ...createMockBackend().state.device, addresses: [] } },
    });
    await show(mock);

    const self = screen.getByRole('region', { name: 'This device' });
    expect(within(self).getByText('Port')).toBeInTheDocument();
    expect(within(self).getByText('52734')).toBeInTheDocument();
    expect(self).toHaveTextContent('No network address yet. Join a network and one appears here.');
  });

  it('says what to try when nothing is in earshot', async () => {
    const mock = createMockBackend({ state: { nearby: [] } });
    await show(mock);
    expect(
      within(screen.getByRole('region', { name: 'Nearby' })).getByText(
        /No other device found on this network/,
      ),
    ).toBeInTheDocument();
  });
});

describe('spellCode', () => {
  it('spaces the digits and keeps the grouping', () => {
    expect(spellCode('482 913')).toBe('4 8 2, 9 1 3');
  });

  it('copes with a code the engine did not group', () => {
    expect(spellCode('482913')).toBe('4 8 2 9 1 3');
  });
});
