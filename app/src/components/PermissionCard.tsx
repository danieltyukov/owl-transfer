import { useCallback, useEffect, useState } from 'react';

import type { Backend, Permission } from '../backend/types.js';

/**
 * How often a paused phone asks again.
 *
 * Long enough that the JNI round trip costs nothing on the one screen that is
 * waiting, short enough that coming back from Android's settings screen feels
 * like the app noticed rather than like it was told.
 */
const RECHECK_MS = 3_000;

/**
 * Why an action fails while the engine is paused.
 *
 * Android is the only platform that pauses and storage is the only reason, so
 * one sentence covers every place that has to say no.
 */
export const PAUSED_NOTE =
  'Sync is paused until storage access is allowed, so nothing can be added yet.';

export interface StorageAccess {
  /** Null until the first answer lands, and after one that failed. */
  permission: Permission | null;
  /** Opens Android's own screen, and asks again on the way back. */
  openSettings: () => void;
}

/*
 * Android's all-files permission, which this app cannot sync without.
 *
 * The folder is /storage/emulated/0/OwlTransfer so that every file manager on
 * the phone can see it, and reaching it from outside the app's own sandbox
 * needs MANAGE_EXTERNAL_STORAGE. The grant happens on a system screen, so the
 * app only ever learns the result, and it learns it by asking again.
 *
 * This lives at the root rather than inside the pane that shows the card. A
 * grant made while any other pane is open used to leave the engine paused until
 * the app was relaunched, because the only thing watching for it was unmounted.
 */
export function useStorageAccess(backend: Backend, paused: boolean): StorageAccess {
  const [permission, setPermission] = useState<Permission | null>(null);

  const check = useCallback(() => {
    void backend.allFilesPermission().then(
      next => {
        setPermission(next);
        // The engine started paused because the folder was out of reach. This
        // is the moment it stops being.
        if (next === 'granted') void backend.setPaused(false).catch(() => undefined);
      },
      () => setPermission(null),
    );
  }, [backend]);

  useEffect(() => {
    check();
    // The system screen is another activity, so coming back is the moment the
    // answer most often has changed.
    //
    // Coming back has to be heard two ways. Android's WebView does not fire
    // `focus` when its activity resumes: `document.hasFocus()` stays false
    // until something is tapped, so on a phone the card would go on saying
    // "Not allowed", with sync still paused, until the person happened to
    // press something. What it does fire is the page visibility change, which
    // is the one that matters here. A desktop window fires `focus` and not
    // that, so both are listened for and the check is cheap enough to run
    // twice where both arrive.
    const woken = (): void => {
      if (!document.hidden) check();
    };
    window.addEventListener('focus', check);
    document.addEventListener('visibilitychange', woken);
    return () => {
      window.removeEventListener('focus', check);
      document.removeEventListener('visibilitychange', woken);
    };
  }, [check]);

  // Neither event is a guarantee. The grant can be made in Android's settings
  // beside this app in split screen, or by a launcher shortcut that never hides
  // the WebView, and then nothing at all fires. A phone that is paused is a
  // phone waiting on this one answer, so while it waits it asks; once it is
  // running there is nothing left to ask about and the timer goes.
  useEffect(() => {
    if (backend.platform !== 'android' || !paused) return undefined;
    const timer = window.setInterval(check, RECHECK_MS);
    return () => window.clearInterval(timer);
  }, [backend, paused, check]);

  const openSettings = useCallback(() => {
    void backend.openAllFilesSettings().then(check, check);
  }, [backend, check]);

  return { permission, openSettings };
}

export interface PermissionCardProps {
  permission: Permission | null;
  onOpenSettings: () => void;
}

/*
 * What the person has to do, said on whichever screen they are on.
 *
 * Nothing on a desktop, and nothing before the first answer: a card that
 * appears and then admits it was not needed is worse than one frame without it.
 */
export function PermissionCard({ permission, onOpenSettings }: PermissionCardProps) {
  if (permission === 'not-applicable' || permission === null) return null;

  const granted = permission === 'granted';

  return (
    <section className="panel" aria-labelledby="storage-access">
      <p className="panel-title" id="storage-access">
        Storage access
      </p>
      <p className="panel-note">
        {granted
          ? 'OwlTransfer can read and write the sync folder.'
          : 'Android keeps the sync folder out of reach until you allow access to all files. Syncing is paused until then.'}
      </p>
      <div className="settings-row">
        <span className="settings-value">{granted ? 'Allowed' : 'Not allowed'}</span>
        {granted ? null : (
          <button type="button" className="button button-primary" onClick={onOpenSettings}>
            Open Android settings
          </button>
        )}
      </div>
    </section>
  );
}
