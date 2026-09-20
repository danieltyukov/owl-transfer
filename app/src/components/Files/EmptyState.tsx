import './EmptyState.css';

export interface EmptyStateProps {
  /** True at the top of the sync folder, where the app has more to say. */
  root: boolean;
  hasPeers: boolean;
  onAdd: () => void;
  onNewFolder: () => void;
  onPairDevice: () => void;
}

/*
 * An empty folder, which on first run is the whole app.
 *
 * Not a wizard. The sentence says what will happen when a file lands here,
 * because that is the thing a person cannot see for themselves yet, and the two
 * buttons are the two ways to make it happen.
 *
 * The second card only appears with nothing paired. Sync with no other device
 * is the one state where adding a file does not do what the sentence promises.
 */
export function EmptyState({ root, hasPeers, onAdd, onNewFolder, onPairDevice }: EmptyStateProps) {
  return (
    <div className="empty">
      {root ? (
        <>
          <p className="prose empty-line">
            Nothing here yet. Add a file, or drop one in, and it will be on your other device in a
            moment.
          </p>
          <div className="empty-actions">
            <button type="button" className="button button-primary button-tall" onClick={onAdd}>
              Add files
            </button>
            <button type="button" className="button button-tall" onClick={onNewFolder}>
              New folder
            </button>
          </div>
          {hasPeers ? null : (
            <div className="panel empty-hint">
              <p className="panel-title">Pair your other device</p>
              <p className="panel-note">
                Nothing is paired yet, so files added here stay on this device.
              </p>
              <div>
                <button type="button" className="button" onClick={onPairDevice}>
                  Open Devices
                </button>
              </div>
            </div>
          )}
        </>
      ) : (
        <p className="prose empty-line">This folder is empty.</p>
      )}
    </div>
  );
}
