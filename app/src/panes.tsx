import { DevicesGlyph, FolderGlyph, GearGlyph } from './icons/glyphs.js';

/** The three places the app has. The sidebar and the phone's tabs share them. */
export type Pane = 'files' | 'devices' | 'settings';

export const PANES: ReadonlyArray<readonly [Pane, string]> = [
  ['files', 'Files'],
  ['devices', 'Devices'],
  ['settings', 'Settings'],
];

export function PaneGlyph({ pane }: { pane: Pane }) {
  if (pane === 'files') return <FolderGlyph />;
  if (pane === 'devices') return <DevicesGlyph />;
  return <GearGlyph />;
}
