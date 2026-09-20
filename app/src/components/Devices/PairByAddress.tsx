import { useId, useState } from 'react';

export interface PairByAddressProps {
  onConnect: (host: string, port: number) => void;
}

const DEFAULT_PORT = '52734';

/*
 * The way in when discovery cannot work.
 *
 * A network with client isolation, or an emulator behind a NAT, will never
 * carry the broadcast beacon. Typing the other device's address is the way
 * round it, and only one side ever has to be able to dial: once the connection
 * is up it carries sync both ways.
 */
export function PairByAddress({ onConnect }: PairByAddressProps) {
  const [host, setHost] = useState('');
  const [port, setPort] = useState(DEFAULT_PORT);
  const hostId = useId();
  const portId = useId();

  const portNumber = Number(port);
  const ready =
    host.trim() !== '' && Number.isInteger(portNumber) && portNumber > 0 && portNumber < 65536;

  return (
    <form
      className="panel"
      onSubmit={event => {
        event.preventDefault();
        if (ready) onConnect(host.trim(), portNumber);
      }}
    >
      <p className="panel-title">Pair by address</p>
      <p className="panel-note">
        For a network that blocks broadcasts, or an emulator. The other device shows its address
        under This device.
      </p>

      <div className="address-row">
        <span className="address-field">
          <label className="address-label" htmlFor={hostId}>
            Address
          </label>
          <input
            id={hostId}
            className="input mono"
            value={host}
            placeholder="192.168.1.31"
            autoComplete="off"
            spellCheck={false}
            onChange={event => setHost(event.target.value)}
          />
        </span>
        <span className="address-field address-port">
          <label className="address-label" htmlFor={portId}>
            Port
          </label>
          <input
            id={portId}
            className="input mono"
            inputMode="numeric"
            value={port}
            onChange={event => setPort(event.target.value)}
          />
        </span>
        <button type="submit" className="button" disabled={!ready}>
          Connect
        </button>
      </div>
    </form>
  );
}
