import { useEffect, useState } from 'react';

import type { Backend, State } from './backend/types.js';

/*
 * The engine's picture of itself, in React.
 *
 * One subscription for the whole app, held at the root and passed down. The
 * engine sends a full `State` on every change rather than a diff, so there is
 * nothing to merge and no local copy to fall behind: what is on screen is
 * whatever arrived last.
 *
 * `null` means the first snapshot has not landed yet, which in practice is one
 * frame on the desktop and a little longer on a phone that is still opening
 * the folder.
 */
export function useBackendState(backend: Backend): State | null {
  const [state, setState] = useState<State | null>(null);

  useEffect(() => {
    let live = true;
    // Subscribed before asked, so a change that lands between the two is not
    // lost. The snapshot only applies if nothing newer has arrived.
    const off = backend.onState(next => {
      if (live) setState(next);
    });
    void backend.getState().then(
      snapshot => {
        if (live) setState(current => current ?? snapshot);
      },
      () => undefined,
    );
    return () => {
      live = false;
      off();
    };
  }, [backend]);

  return state;
}
