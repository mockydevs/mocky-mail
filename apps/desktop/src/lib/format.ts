const KB = 1024;
const MB = KB * 1024;
const GB = MB * 1024;
const DAY_MS = 86_400_000;

function trimmed(value: number): string {
  return value < 10 ? (Math.round(value * 10) / 10).toString() : Math.round(value).toString();
}

/** Human file size: "412 KB", "1.8 MB", "28 MB". */
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return '0 B';
  if (bytes < KB) return `${Math.round(bytes)} B`;
  if (bytes < MB) return `${trimmed(bytes / KB)} KB`;
  if (bytes < GB) return `${trimmed(bytes / MB)} MB`;
  return `${trimmed(bytes / GB)} GB`;
}

interface DateParts {
  year: number;
  month: number;
  day: number;
  weekday: string;
  monthName: string;
  hour: string;
  minute: string;
}

function partsIn(date: Date, timeZone: string | undefined): DateParts {
  const fmt = new Intl.DateTimeFormat('en-GB', {
    timeZone,
    year: 'numeric',
    month: 'numeric',
    day: 'numeric',
    weekday: 'short',
    hour: '2-digit',
    minute: '2-digit',
    hourCycle: 'h23',
  });
  const named = new Intl.DateTimeFormat('en-GB', { timeZone, month: 'short' });
  const get = (type: Intl.DateTimeFormatPartTypes): string =>
    fmt.formatToParts(date).find((p) => p.type === type)?.value ?? '';
  return {
    year: Number(get('year')),
    month: Number(get('month')),
    day: Number(get('day')),
    weekday: get('weekday'),
    monthName: named.format(date),
    hour: get('hour'),
    minute: get('minute'),
  };
}

function dayNumber(p: DateParts): number {
  return Date.UTC(p.year, p.month - 1, p.day) / DAY_MS;
}

/**
 * Message-list time label relative to `now`: "09:42", "Yesterday", "Tue",
 * or "3 Nov". Future dates (clock skew) show the time.
 */
export function formatListTime(iso: string, now: Date, timeZone?: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return '';
  const d = partsIn(date, timeZone);
  const n = partsIn(now, timeZone);
  const daysAgo = dayNumber(n) - dayNumber(d);
  if (daysAgo <= 0) return `${d.hour}:${d.minute}`;
  if (daysAgo === 1) return 'Yesterday';
  if (daysAgo < 7) return d.weekday;
  if (d.year === n.year) return `${d.day} ${d.monthName}`;
  return `${d.day} ${d.monthName} ${d.year}`;
}

/** Reading-pane date: "Thu 13 Nov, 09:42". */
export function formatLongDate(iso: string, timeZone?: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return '';
  const d = partsIn(date, timeZone);
  return `${d.weekday} ${d.day} ${d.monthName}, ${d.hour}:${d.minute}`;
}

/** Clock time only: "10:02". */
export function formatClock(iso: string, timeZone?: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return '';
  const d = partsIn(date, timeZone);
  return `${d.hour}:${d.minute}`;
}
