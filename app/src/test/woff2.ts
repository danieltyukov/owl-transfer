import { brotliDecompressSync } from 'node:zlib';

/*
 * Just enough woff2 to read a font's name table.
 *
 * This exists so the licence check reads the files that actually ship rather
 * than a manifest written beside them, which could agree with itself while the
 * binaries drifted. The rename asserts the same thing, but it asserts it before
 * the subsetter runs, and the subsetter is told to keep every name record.
 *
 * Shelling out to fontTools would have been shorter and would have silently
 * skipped on any machine without it, which is not a guard. Carried over from
 * to-hoot, which ships its own renamed subsets of the same two families.
 *
 * Only the parts needed are implemented: the header, enough of the table
 * directory to find where `name` begins in the decompressed stream, and the
 * name table itself. woff2 transforms only `glyf` and `loca`, so `name` is
 * always stored whole.
 *
 * Reference: W3C WOFF2, sections 4 (header) and 5 (table directory).
 */

export interface NameRecord {
  platformID: number;
  encodingID: number;
  languageID: number;
  nameID: number;
  text: string;
}

/** The 63 tags woff2 can encode in five bits, in the order the spec fixes. */
const KNOWN_TAGS = [
  'cmap', 'head', 'hhea', 'hmtx', 'maxp', 'name', 'OS/2', 'post', 'cvt ', 'fpgm',
  'glyf', 'loca', 'prep', 'CFF ', 'VORG', 'EBDT', 'EBLC', 'gasp', 'hdmx', 'kern',
  'LTSH', 'PCLT', 'VDMX', 'vhea', 'vmtx', 'BASE', 'GDEF', 'GPOS', 'GSUB', 'EBSC',
  'JSTF', 'MATH', 'CBDT', 'CBLC', 'COLR', 'CPAL', 'SVG ', 'sbix', 'acnt', 'avar',
  'bdat', 'bloc', 'bsln', 'cvar', 'fdsc', 'feat', 'fmtx', 'fvar', 'gvar', 'hsty',
  'just', 'lcar', 'mort', 'morx', 'opbd', 'prop', 'trak', 'Zapf', 'Silf', 'Glat',
  'Gloc', 'Feat', 'Sill',
];

/** woff2's UIntBase128: seven bits per byte, high bit continues. */
function readBase128(buf: Buffer, at: number): [value: number, next: number] {
  let value = 0;
  let i = at;
  for (let read = 0; read < 5; read++) {
    const byte = buf[i++]!;
    value = value * 128 + (byte & 0x7f);
    if ((byte & 0x80) === 0) return [value, i];
  }
  throw new Error('malformed UIntBase128');
}

/** UTF-16BE for the Windows platform, Mac Roman treated as Latin-1. */
function decodeName(bytes: Buffer, platformID: number): string {
  if (platformID === 3 || platformID === 0) return bytes.toString('utf16le').length === 0
    ? ''
    : Buffer.from(bytes).swap16().toString('utf16le');
  return bytes.toString('latin1');
}

function parseNameTable(table: Buffer): NameRecord[] {
  const count = table.readUInt16BE(2);
  const stringOffset = table.readUInt16BE(4);
  const records: NameRecord[] = [];
  for (let i = 0; i < count; i++) {
    const at = 6 + i * 12;
    const length = table.readUInt16BE(at + 8);
    const offset = table.readUInt16BE(at + 10);
    const platformID = table.readUInt16BE(at);
    const bytes = table.subarray(stringOffset + offset, stringOffset + offset + length);
    records.push({
      platformID,
      encodingID: table.readUInt16BE(at + 2),
      languageID: table.readUInt16BE(at + 4),
      nameID: table.readUInt16BE(at + 6),
      text: decodeName(bytes, platformID),
    });
  }
  return records;
}

/** Every name record in a woff2 file. */
export function readNameTable(file: Buffer): NameRecord[] {
  if (file.toString('latin1', 0, 4) !== 'wOF2') throw new Error('not a woff2 file');
  const numTables = file.readUInt16BE(12);

  let at = 48;
  let nameOffset: number | null = null;
  let nameLength = 0;
  let running = 0;

  for (let i = 0; i < numTables; i++) {
    const flags = file[at++]!;
    const index = flags & 0x3f;
    let tag: string;
    if (index === 0x3f) {
      tag = file.toString('latin1', at, at + 4);
      at += 4;
    } else {
      tag = KNOWN_TAGS[index] ?? '????';
    }

    let length: number;
    [length, at] = readBase128(file, at);
    // Only glyf and loca are ever transformed; for those the transformed length
    // follows and is what occupies the stream.
    const transform = (flags >> 6) & 0x03;
    if ((tag === 'glyf' || tag === 'loca') && transform !== 3) {
      [length, at] = readBase128(file, at);
    } else if (tag !== 'glyf' && tag !== 'loca' && transform !== 0) {
      [length, at] = readBase128(file, at);
    }

    if (tag === 'name') {
      nameOffset = running;
      nameLength = length;
    }
    running += length;
  }

  if (nameOffset === null) throw new Error('no name table');
  const font = brotliDecompressSync(file.subarray(at));
  return parseNameTable(font.subarray(nameOffset, nameOffset + nameLength));
}
