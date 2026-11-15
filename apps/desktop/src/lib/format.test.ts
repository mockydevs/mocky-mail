import { dayGroup, formatBytes, formatClock, formatListTime, formatLongDate } from './format';

const s = (iso: string) => Date.parse(iso) / 1000;

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
  const now = s('2026-11-13T10:15:00Z');

  it('shows the time for today', () => {
    expect(formatListTime(s('2026-11-13T06:42:00Z'), now, 'UTC')).toBe('06:42');
  });

  it('shows Yesterday, then the weekday, then the date', () => {
    expect(formatListTime(s('2026-11-12T23:59:00Z'), now, 'UTC')).toBe('Yesterday');
    expect(formatListTime(s('2026-11-10T12:00:00Z'), now, 'UTC')).toBe('Tue');
    expect(formatListTime(s('2026-11-03T08:00:00Z'), now, 'UTC')).toBe('3 Nov');
    expect(formatListTime(s('2025-11-03T08:00:00Z'), now, 'UTC')).toBe('3 Nov 2025');
  });

  it('uses the calendar day of the given time zone', () => {
    // 22:30 UTC on the 12th is already the 13th in Nairobi (UTC+3).
    expect(formatListTime(s('2026-11-12T22:30:00Z'), now, 'Africa/Nairobi')).toBe('01:30');
    expect(formatListTime(s('2026-11-12T22:30:00Z'), now, 'UTC')).toBe('Yesterday');
  });

  it('handles future, missing and invalid dates', () => {
    expect(formatListTime(s('2026-11-14T09:00:00Z'), now, 'UTC')).toBe('09:00');
    expect(formatListTime(null, now, 'UTC')).toBe('');
    expect(formatListTime(Number.NaN, now, 'UTC')).toBe('');
  });
});

describe('formatLongDate and formatClock', () => {
  it('formats in the given zone', () => {
    expect(formatLongDate(s('2026-11-13T06:42:00Z'), 'Africa/Nairobi')).toBe('Fri 13 Nov 2026, 09:42');
    expect(formatClock(s('2026-11-13T07:02:00Z'), 'Africa/Nairobi')).toBe('10:02');
  });

  it('returns nothing for missing dates', () => {
    expect(formatLongDate(null)).toBe('');
    expect(formatClock(null)).toBe('');
  });
});

describe('dayGroup', () => {
  const now = s('2026-11-13T10:15:00Z'); // a Friday

  it('names today, yesterday and the past week by day', () => {
    expect(dayGroup(s('2026-11-13T01:00:00Z'), now, 'UTC')).toBe('Today');
    expect(dayGroup(s('2026-11-12T23:00:00Z'), now, 'UTC')).toBe('Yesterday');
    expect(dayGroup(s('2026-11-09T09:00:00Z'), now, 'UTC')).toBe('Monday');
  });

  it('groups older mail by month, with the year when it differs', () => {
    expect(dayGroup(s('2026-11-02T09:00:00Z'), now, 'UTC')).toBe('November');
    expect(dayGroup(s('2026-10-20T09:00:00Z'), now, 'UTC')).toBe('October');
    expect(dayGroup(s('2025-12-24T09:00:00Z'), now, 'UTC')).toBe('December 2025');
  });

  it('follows the time zone and handles missing dates', () => {
    // 23:30 UTC on the 12th is already the 13th in Nairobi.
    expect(dayGroup(s('2026-11-12T23:30:00Z'), now, 'Africa/Nairobi')).toBe('Today');
    expect(dayGroup(null, now, 'UTC')).toBe('Earlier');
  });
});
