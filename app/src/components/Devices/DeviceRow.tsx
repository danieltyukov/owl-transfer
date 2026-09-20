import type { ReactNode } from 'react';

import type { DeviceKind } from '../../backend/types.js';
import { DesktopGlyph, PhoneGlyph } from '../../icons/glyphs.js';

export interface DeviceRowProps {
  name: string;
  kind: DeviceKind;
  /** Where it is, or when it was last heard from. */
  detail: string;
  connected?: boolean;
  children?: ReactNode;
}

/*
 * One device, paired or merely seen.
 *
 * The dot is the same language the sidebar and the status strip speak: filled
 * green is here, an outline is paired but not answering, and nothing at all is
 * a device that has never been paired.
 */
export function DeviceRow({ name, kind, detail, connected, children }: DeviceRowProps) {
  return (
    <li className="device">
      <span className="device-glyph" aria-hidden="true">
        {kind === 'phone' ? <PhoneGlyph /> : <DesktopGlyph />}
      </span>
      <span className="device-lines">
        <span className="device-name">{name}</span>
        <span className="device-detail mono">{detail}</span>
      </span>
      {connected === undefined ? null : (
        <span className="device-dot" data-connected={connected ? '' : undefined} />
      )}
      <span className="device-actions">{children}</span>
    </li>
  );
}
