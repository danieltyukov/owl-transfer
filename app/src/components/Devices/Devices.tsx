import { useState } from 'react';

import type { Backend, PeerInfo, State } from '../../backend/types.js';
import { formatRelative } from '../../format.js';
import { Dialog } from '../Dialog.js';
import { DeviceRow } from './DeviceRow.js';
import { PairByAddress } from './PairByAddress.js';
import { PairingCard } from './PairingCard.js';
import './Devices.css';

export interface DevicesProps {
  backend: Backend;
  state: State;
  onError: (message: string) => void;
}

/*
 * Who this device is paired with, who it can see, and how to reach one it
 * cannot.
 *
 * A pending request goes at the top and takes the whole width, because it
 * expires: 60 seconds on the engine's side, and it is the only thing on this
 * screen that will be gone if it is not answered.
 */
export function Devices({ backend, state, onError }: DevicesProps) {
  const [forgetting, setForgetting] = useState<PeerInfo | null>(null);
  const now = Date.now();

  const peerDetail = (peer: PeerInfo): string => {
    if (peer.connected) return peer.address ?? 'Connected';
    if (peer.lastSeenMs === null) return 'Not seen yet';
    return `Last seen ${formatRelative(peer.lastSeenMs, now)}`;
  };

  return (
    <>
      <div className="pane-title">
        <h1>Devices</h1>
      </div>

      <div className="pane-body">
        <div className="pane-column">
          {state.pendingPairing === null ? null : (
            <PairingCard
              pairing={state.pendingPairing}
              onRespond={accept => {
                const id = state.pendingPairing?.id;
                if (id === undefined) return;
                void backend
                  .respondToPairing(id, accept)
                  .catch(() => onError('That pairing could not be answered.'));
              }}
            />
          )}

          <section className="devices-group" aria-labelledby="devices-paired">
            <h2 className="section-title" id="devices-paired">
              Paired
            </h2>
            {state.peers.length === 0 ? (
              <p className="panel-note">
                Nothing is paired yet. Pair a device below and this folder is on both.
              </p>
            ) : (
              <ul className="devices-list">
                {state.peers.map(peer => (
                  <DeviceRow
                    key={peer.id}
                    name={peer.name}
                    kind={peer.kind}
                    detail={peerDetail(peer)}
                    connected={peer.connected}
                  >
                    <button
                      type="button"
                      className="button button-danger"
                      onClick={() => setForgetting(peer)}
                    >
                      Forget
                    </button>
                  </DeviceRow>
                ))}
              </ul>
            )}
          </section>

          <section className="devices-group" aria-labelledby="devices-nearby">
            <h2 className="section-title" id="devices-nearby">
              Nearby
            </h2>
            {state.nearby.length === 0 ? (
              <p className="panel-note">
                No other device found on this network. Make sure both are on the same Wi-Fi, or
                pair by address.
              </p>
            ) : (
              <ul className="devices-list">
                {state.nearby.map(device => (
                  <DeviceRow
                    key={device.id}
                    name={device.name}
                    kind={device.kind}
                    detail={device.address}
                  >
                    <button
                      type="button"
                      className="button button-primary"
                      onClick={() => {
                        void backend
                          .pairWithNearby(device.id)
                          .catch(() => onError(`${device.name} could not be reached.`));
                      }}
                    >
                      Pair
                    </button>
                  </DeviceRow>
                ))}
              </ul>
            )}
          </section>

          <PairByAddress
            onConnect={(host, port) => {
              void backend
                .pairWithAddress(host, port)
                .catch(() => onError(`${host} could not be reached.`));
            }}
          />

          <section className="panel" aria-labelledby="devices-self">
            <p className="panel-title" id="devices-self">
              This device
            </p>
            <dl className="self">
              <dt>Name</dt>
              <dd>{state.device.name}</dd>
              <dt>Id</dt>
              <dd className="mono">{state.device.id.slice(0, 8)}</dd>
              {state.device.addresses.length === 0 ? (
                <>
                  <dt>Port</dt>
                  <dd className="mono">{state.device.port}</dd>
                </>
              ) : (
                <>
                  <dt>{state.device.addresses.length === 1 ? 'Address' : 'Addresses'}</dt>
                  {/*
                    The port is carried on every line rather than given a row of
                    its own. What the other device asks for is one string, and a
                    person reading this out should not have to assemble it.
                  */}
                  <dd>
                    <ul className="self-addresses">
                      {state.device.addresses.map(address => (
                        <li key={address} className="mono">
                          {address}:{state.device.port}
                        </li>
                      ))}
                    </ul>
                  </dd>
                </>
              )}
            </dl>
            <p className="panel-note">
              {state.device.addresses.length === 0
                ? 'No network address yet. Join a network and one appears here.'
                : 'Type one of these on the other device to pair by address.'}
            </p>
          </section>
        </div>
      </div>

      {forgetting === null ? null : (
        <Dialog
          title={`Forget ${forgetting.name}?`}
          description="It stops syncing. Nothing already in the folder is deleted on either device."
          submitLabel="Forget"
          danger
          onSubmit={() => {
            const peer = forgetting;
            setForgetting(null);
            void backend
              .forgetPeer(peer.id)
              .catch(() => onError(`${peer.name} could not be forgotten.`));
          }}
          onClose={() => setForgetting(null)}
        />
      )}
    </>
  );
}
