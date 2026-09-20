import { describe, expect, it } from 'vitest';

import { formatBytes, formatRate, formatRelative, plural } from './format.js';

describe('formatBytes', () => {
  it('counts in thousands, the way every file manager does', () => {
    expect(formatBytes(0)).toBe('0 B');
    expect(formatBytes(999)).toBe('999 B');
    expect(formatBytes(1_000)).toBe('1.0 kB');
    expect(formatBytes(1_204)).toBe('1.2 kB');
  });

  it('drops the decimal above ten, so a column of sizes keeps one width', () => {
    expect(formatBytes(3_140_000)).toBe('3.1 MB');
    expect(formatBytes(48_200_000)).toBe('48 MB');
    expect(formatBytes(2_400_000_000)).toBe('2.4 GB');
  });

  it('reads a missing or impossible size as nothing rather than as NaN', () => {
    expect(formatBytes(-1)).toBe('0 B');
    expect(formatBytes(Number.NaN)).toBe('0 B');
  });
});

describe('formatRate', () => {
  it('is a size per second', () => {
    expect(formatRate(12_400_000)).toBe('12 MB/s');
    expect(formatRate(0)).toBe('0 B/s');
  });
});

describe('formatRelative', () => {
  // Local, not UTC: the dates below are compared against a local calendar, so
  // a fixed instant would name a different day in a far enough timezone.
  const now = new Date(2026, 8, 20, 12, 0, 0).getTime();

  it('calls the last half minute now', () => {
    expect(formatRelative(now - 2_000, now)).toBe('just now');
    expect(formatRelative(now - 44_000, now)).toBe('just now');
  });

  it('counts minutes and then hours', () => {
    expect(formatRelative(now - 2 * 60_000, now)).toBe('2 min ago');
    expect(formatRelative(now - 50 * 60_000, now)).toBe('50 min ago');
    expect(formatRelative(now - 3 * 3_600_000, now)).toBe('3 hours ago');
  });

  it('names yesterday rather than counting back 26 hours', () => {
    expect(formatRelative(now - 26 * 3_600_000, now)).toBe('yesterday');
    expect(formatRelative(now - 4 * 86_400_000, now)).toBe('4 days ago');
  });

  it('gives a date once a week has passed', () => {
    expect(formatRelative(new Date(2026, 2, 12, 9, 0, 0).getTime(), now)).toBe('12 Mar');
    expect(formatRelative(new Date(2025, 10, 3, 9, 0, 0).getTime(), now)).toBe('3 Nov 2025');
  });

  it('reads a clock that runs ahead of ours as now', () => {
    // A phone back from sleep can stamp a file a few seconds into our future.
    expect(formatRelative(now + 30_000, now)).toBe('just now');
  });
});

describe('plural', () => {
  it('agrees with its count', () => {
    expect(plural(1, 'file')).toBe('1 file');
    expect(plural(3, 'file')).toBe('3 files');
    expect(plural(0, 'device')).toBe('0 devices');
  });
});
