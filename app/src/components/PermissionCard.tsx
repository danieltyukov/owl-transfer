import { useCallback, useEffect, useState } from 'react';

import type { Backend, Permission } from '../backend/types.js';

export interface PermissionCardProps {
  backend: Backend;
}

/*
 * Android's all-files permission, which this app cannot sync without.
 *
 * The folder is /storage/emulated/0/OwlTransfer so that every file manager on
 * the phone can see it, and reaching it from outside the app's own sandbox
 * needs MANAGE_EXTERNAL_STORAGE. The grant happens on a system screen, so the
 * app only ever learns the result, and it learns it by asking again when the
 * window comes back.
 */
export function PermissionCard({ backend }: PermissionCardProps) {
  const [permission, setPermission] = useState<Permission | null>(null);

  const check = useCallback(() => {
    void backend.allFilesPermission().then(
      next => {
        setPermission(next);
        // The engine started paused because the folder was out of reach. This
        // is the moment it stops being, and nothing else is watching for it.
        if (next === 'granted') void backend.setPaused(false).catch(() => undefined);
      },
      () => setPermission(null),
    );
  }, [backend]);

  useEffect(() => {
    check();
    // The system screen is another activity, so coming back is the only moment
    // the answer can have changed.
    window.addEventListener('focus', check);
    return () => window.removeEventListener('focus', check);
  }, [check]);

  if (permission === 'not-applicable' || permission === null) return null;

  const granted = permission === 'granted';

  return (
    <section className="panel" aria-labelledby="settings-storage">
      <p className="panel-title" id="settings-storage">
        Storage access
      </p>
      <p className="panel-note">
        {granted
          ? 'Owl Transfer can read and write the sync folder.'
          : 'Android keeps the sync folder out of reach until you allow access to all files. Syncing is paused until then.'}
      </p>
      <div className="settings-row">
        <span className="settings-value">{granted ? 'Allowed' : 'Not allowed'}</span>
        {granted ? null : (
          <button
            type="button"
            className="button button-primary"
            onClick={() => {
              void backend.openAllFilesSettings().then(check, check);
            }}
          >
            Open Android settings
          </button>
        )}
      </div>
    </section>
  );
}
