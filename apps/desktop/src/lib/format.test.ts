import { formatBytes, formatClock, formatListTime, formatLongDate } from './format';

describe('formatBytes', () => {
  it.each([
    [0, '0 B'],
    [512, '512 B'],
    [96 * 1024, '96 KB'],
    [1.84 * 1024 * 1024, '1.8 MB'],
    [28 * 1024 * 1024, '28 MB'],
    [3 * 1024 ** 3, '3 GB'],
  ])('%d bytes is %s', (bytes, label) => {
    expect(formatBytes(bytes)).toBe(label);
  });

  it('treats invalid input as zero', () => {
    expect(formatBytes(-5)).toBe('0 B');
    expect(formatBytes(Number.NaN)).toBe('0 B');
  });
});

describe('formatListTime', () => {
  const now = new Date('2026-11-13T10:15:00Z');

  it('shows the time for today', () => {
    expect(formatListTime('2026-11-13T06:42:00Z', now, 'UTC')).toBe('06:42');
  });

  it('shows Yesterday, then the weekday, then the date', () => {
    expect(formatListTime('2026-11-12T23:59:00Z', now, 'UTC')).toBe('Yesterday');
    expect(formatListTime('2026-11-10T12:00:00Z', now, 'UTC')).toBe('Tue');
    expect(formatListTime('2026-11-03T08:00:00Z', now, 'UTC')).toBe('3 Nov');
    expect(formatListTime('2025-11-03T08:00:00Z', now, 'UTC')).toBe('3 Nov 2025');
  });

  it('uses the calendar day of the given time zone', () => {
    // 22:30 UTC on the 12th is already the 13th in Nairobi (UTC+3).
    expect(formatListTime('2026-11-12T22:30:00Z', now, 'Africa/Nairobi')).toBe('01:30');
    expect(formatListTime('2026-11-12T22:30:00Z', now, 'UTC')).toBe('Yesterday');
  });

  it('shows the time for future dates and nothing for garbage', () => {
    expect(formatListTime('2026-11-14T09:00:00Z', now, 'UTC')).toBe('09:00');
    expect(formatListTime('not a date', now, 'UTC')).toBe('');
  });
});

describe('formatLongDate and formatClock', () => {
  it('formats in the given zone', () => {
    expect(formatLongDate('2026-11-13T06:42:00Z', 'Africa/Nairobi')).toBe('Fri 13 Nov, 09:42');
    expect(formatClock('2026-11-13T07:02:00Z', 'Africa/Nairobi')).toBe('10:02');
  });

  it('returns nothing for garbage', () => {
    expect(formatLongDate('nope')).toBe('');
    expect(formatClock('nope')).toBe('');
  });
});
