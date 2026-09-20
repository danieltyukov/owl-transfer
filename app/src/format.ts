/*
 * Every number the interface puts in front of a person.
 *
 * Sizes are decimal, not binary: the folder this app keeps is read in a file
 * manager, and every file manager on the three platforms it targets counts in
 * thousands. Showing 2.3 MiB beside the same file's 2.4 MB elsewhere is a bug
 * report waiting to happen.
 */

const UNITS = ['B', 'kB', 'MB', 'GB', 'TB'] as const;

/**
 * "0 B", "1.2 kB", "48 MB". One decimal under ten and none above it, so a
 * column of sizes is the same width whatever is in it.
 */
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return '0 B';
  let value = bytes;
  let unit = 0;
  while (value >= 1000 && unit < UNITS.length - 1) {
    value /= 1000;
    unit += 1;
  }
  if (unit === 0) return `${Math.round(value)} B`;
  return `${value < 10 ? value.toFixed(1) : Math.round(value)} ${UNITS[unit]}`;
}

/** "12 MB/s". The rate the status strip reports while files are in flight. */
export function formatRate(bytesPerSec: number): string {
  return `${formatBytes(bytesPerSec)}/s`;
}

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;

const MONTHS = [
  'Jan',
  'Feb',
  'Mar',
  'Apr',
  'May',
  'Jun',
  'Jul',
  'Aug',
  'Sep',
  'Oct',
  'Nov',
  'Dec',
] as const;

/**
 * "just now", "2 min ago", "yesterday", "12 Mar".
 *
 * A time in the future is a clock that disagrees with the other device's, which
 * happens on a phone that has just come back from sleep. It reads as now rather
 * than as "in 3 minutes", which would be a puzzle rather than information.
 */
export function formatRelative(ms: number, now: number = Date.now()): string {
  const ago = now - ms;
  if (ago < 45 * 1000) return 'just now';
  if (ago < 90 * 1000) return '1 min ago';
  if (ago < HOUR) return `${Math.round(ago / MINUTE)} min ago`;
  if (ago < 90 * MINUTE) return '1 hour ago';
  if (ago < DAY) return `${Math.round(ago / HOUR)} hours ago`;
  if (ago < 2 * DAY) return 'yesterday';
  if (ago < 7 * DAY) return `${Math.floor(ago / DAY)} days ago`;

  const then = new Date(ms);
  const day = `${then.getDate()} ${MONTHS[then.getMonth()]}`;
  return then.getFullYear() === new Date(now).getFullYear()
    ? day
    : `${day} ${then.getFullYear()}`;
}

/** "1 file", "3 files". The status strip and the summary both count things. */
export function plural(count: number, singular: string, many = `${singular}s`): string {
  return `${count} ${count === 1 ? singular : many}`;
}
