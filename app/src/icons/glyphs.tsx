import type { ReactNode, SVGProps } from 'react';

/*
 * The interface glyphs. All 16x16, all `currentColor`, all decorative: every
 * one of them sits inside a control or a row that carries its own accessible
 * name, so a second name here would be read out twice.
 *
 * One grid, one stroke range (1.3 to 1.6), round caps. A glyph that arrives
 * from an icon set at a different weight is visible immediately in a row of
 * these, which is the reason they are drawn here rather than installed.
 */

type GlyphProps = Omit<SVGProps<SVGSVGElement>, 'viewBox' | 'width' | 'height'>;

function Glyph({ children, ...rest }: GlyphProps) {
  return (
    <svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true" focusable="false" {...rest}>
      {children}
    </svg>
  );
}

/* The window controls, for the desktop shell that draws no native title bar. */

export function MinimizeGlyph(props: GlyphProps) {
  return (
    <Glyph {...props}>
      <path
        d="M4 10.5 H12"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.3"
        strokeLinecap="round"
      />
    </Glyph>
  );
}

export function MaximizeGlyph(props: GlyphProps) {
  return (
    <Glyph {...props}>
      <rect
        x="4"
        y="4"
        width="8"
        height="8"
        rx="1"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.3"
      />
    </Glyph>
  );
}

export function CloseGlyph(props: GlyphProps) {
  return (
    <Glyph {...props}>
      <path
        d="M4.5 4.5 L11.5 11.5 M11.5 4.5 L4.5 11.5"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.3"
        strokeLinecap="round"
      />
    </Glyph>
  );
}

/* Navigation: the three places the app has. */

/** Files: a folder, which is what the app is. */
export function FolderGlyph(props: GlyphProps) {
  return (
    <Glyph {...props}>
      <path
        d="M2 4.5 A1.5 1.5 0 0 1 3.5 3 H6.2 L7.7 5 H12.5 A1.5 1.5 0 0 1 14 6.5 V11.5 A1.5 1.5 0 0 1 12.5 13 H3.5 A1.5 1.5 0 0 1 2 11.5 Z"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.4"
        strokeLinejoin="round"
      />
    </Glyph>
  );
}

/** Devices: a screen with a phone in front of it. The pair, which is the app. */
export function DevicesGlyph(props: GlyphProps) {
  return (
    <Glyph {...props}>
      <path
        d="M2.75 4 A1 1 0 0 1 3.75 3 H12.25 A1 1 0 0 1 13.25 4 V8.5"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.4"
        strokeLinecap="round"
      />
      <path
        d="M2.75 4 V9.5 A1 1 0 0 0 3.75 10.5 H7"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.4"
        strokeLinecap="round"
      />
      <rect
        x="9"
        y="7"
        width="5"
        height="7"
        rx="1.2"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.4"
      />
    </Glyph>
  );
}

export function GearGlyph(props: GlyphProps) {
  return (
    <Glyph {...props}>
      <circle cx="8" cy="8" r="2.2" fill="none" stroke="currentColor" strokeWidth="1.4" />
      <path
        d="M8 2.2 V3.6 M8 12.4 V13.8 M2.2 8 H3.6 M12.4 8 H13.8 M3.9 3.9 L4.9 4.9 M11.1 11.1 L12.1 12.1 M12.1 3.9 L11.1 4.9 M4.9 11.1 L3.9 12.1"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.4"
        strokeLinecap="round"
      />
    </Glyph>
  );
}

/* Actions. */

export function PlusGlyph(props: GlyphProps) {
  return (
    <Glyph {...props}>
      <path
        d="M8 3.5 V12.5 M3.5 8 H12.5"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.6"
        strokeLinecap="round"
      />
    </Glyph>
  );
}

/** New folder: the folder, with the plus where its contents would be. */
export function FolderPlusGlyph(props: GlyphProps) {
  return (
    <Glyph {...props}>
      <path
        d="M2 4.5 A1.5 1.5 0 0 1 3.5 3 H6.2 L7.7 5 H12.5 A1.5 1.5 0 0 1 14 6.5 V11.5 A1.5 1.5 0 0 1 12.5 13 H3.5 A1.5 1.5 0 0 1 2 11.5 Z"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.4"
        strokeLinejoin="round"
      />
      <path
        d="M8 7.3 V10.7 M6.3 9 H9.7"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.4"
        strokeLinecap="round"
      />
    </Glyph>
  );
}

export function ChevronGlyph(props: GlyphProps) {
  return (
    <Glyph {...props}>
      <path
        d="M6 4 L10 8 L6 12"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.6"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </Glyph>
  );
}

/** The row's overflow control. Three dots, upright, so it reads as a menu. */
export function MoreGlyph(props: GlyphProps) {
  return (
    <Glyph {...props}>
      <circle cx="8" cy="3.6" r="1.35" fill="currentColor" />
      <circle cx="8" cy="8" r="1.35" fill="currentColor" />
      <circle cx="8" cy="12.4" r="1.35" fill="currentColor" />
    </Glyph>
  );
}

export function CheckGlyph(props: GlyphProps) {
  return (
    <Glyph {...props}>
      <path
        d="M3.6 8.4 L6.6 11.4 L12.4 4.9"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.8"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </Glyph>
  );
}

/** Leaves the app: the repository link in About. */
export function ExternalGlyph(props: GlyphProps) {
  return (
    <Glyph {...props}>
      <path
        d="M12.5 9 V12 A1.5 1.5 0 0 1 11 13.5 H4 A1.5 1.5 0 0 1 2.5 12 V5 A1.5 1.5 0 0 1 4 3.5 H7"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.4"
        strokeLinecap="round"
      />
      <path
        d="M9.5 2.5 H13.5 V6.5 M13.5 2.5 L7.5 8.5"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.4"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </Glyph>
  );
}

/* A device's kind, in a peer row and in the beacon list. */

export function PhoneGlyph(props: GlyphProps) {
  return (
    <Glyph {...props}>
      <rect
        x="4.75"
        y="2.25"
        width="6.5"
        height="11.5"
        rx="1.5"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.4"
      />
      <path
        d="M7 11.5 H9"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.4"
        strokeLinecap="round"
      />
    </Glyph>
  );
}

export function DesktopGlyph(props: GlyphProps) {
  return (
    <Glyph {...props}>
      <rect
        x="2.25"
        y="3.25"
        width="11.5"
        height="7.5"
        rx="1.5"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.4"
      />
      <path
        d="M6 13 H10 M8 10.75 V13"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.4"
        strokeLinecap="round"
      />
    </Glyph>
  );
}

/*
 * A file's kind, by extension. Every one of these is the same sheet with a
 * folded corner and a different thing inside it, so a list of mixed files reads
 * as one list rather than as seven icon sets.
 */

function Sheet({ children }: { children?: ReactNode }) {
  return (
    <>
      <path
        d="M3.5 2.5 H9 L12.5 6 V13.5 H3.5 Z"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.4"
        strokeLinejoin="round"
      />
      <path
        d="M9 2.5 V6 H12.5"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.4"
        strokeLinejoin="round"
      />
      {children}
    </>
  );
}

export function FileGlyph(props: GlyphProps) {
  return (
    <Glyph {...props}>
      <Sheet />
    </Glyph>
  );
}

export function DocumentGlyph(props: GlyphProps) {
  return (
    <Glyph {...props}>
      <Sheet>
        <path
          d="M5.6 8.6 H10.4 M5.6 11 H9"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.3"
          strokeLinecap="round"
        />
      </Sheet>
    </Glyph>
  );
}

export function ImageGlyph(props: GlyphProps) {
  return (
    <Glyph {...props}>
      <Sheet>
        <circle cx="6.4" cy="8.4" r="0.95" fill="currentColor" />
        <path
          d="M5 12.4 L7.8 9.8 L9.4 11.2 L10.8 10 L11.6 10.8"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.3"
          strokeLinecap="round"
          strokeLinejoin="round"
        />
      </Sheet>
    </Glyph>
  );
}

export function AudioGlyph(props: GlyphProps) {
  return (
    <Glyph {...props}>
      <Sheet>
        <path
          d="M6.4 11.4 V8 L10 7.1 V10.5"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.3"
          strokeLinecap="round"
          strokeLinejoin="round"
        />
        <circle cx="5.6" cy="11.5" r="0.95" fill="currentColor" />
        <circle cx="9.2" cy="10.6" r="0.95" fill="currentColor" />
      </Sheet>
    </Glyph>
  );
}

export function VideoGlyph(props: GlyphProps) {
  return (
    <Glyph {...props}>
      <Sheet>
        <path d="M6.6 8.2 L10.4 10.5 L6.6 12.8 Z" fill="currentColor" />
      </Sheet>
    </Glyph>
  );
}

export function ArchiveGlyph(props: GlyphProps) {
  return (
    <Glyph {...props}>
      <Sheet>
        <path
          d="M6.6 7.2 H7.8 M6.6 9 H7.8 M6.6 10.8 H7.8"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.3"
          strokeLinecap="round"
        />
      </Sheet>
    </Glyph>
  );
}
