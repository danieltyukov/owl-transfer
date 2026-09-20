import type { Backend, State } from '../../backend/types.js';

export interface DevicesProps {
  backend: Backend;
  state: State;
}

/** The devices this one is paired with, and the ones it can see. */
export function Devices({ state }: DevicesProps) {
  return (
    <>
      <div className="pane-title">
        <h1>Devices</h1>
      </div>
      <div className="pane-body">
        <div className="pane-column">
          <p className="panel-note">{state.peers.length} paired</p>
        </div>
      </div>
    </>
  );
}
