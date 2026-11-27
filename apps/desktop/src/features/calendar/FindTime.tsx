import { useEffect, useMemo, useState } from 'react';

import { useMailApi } from '../../api/MailApiContext';
import { toAppError } from '../../api/errors';
import type { AppError, FreeBusy, ShowAs } from '../../api/types';
import { Icon } from '../../ui/Icon';
import { Modal } from '../../ui/Modal';
import { addDays, atMinutes, dayText, hhmm, seconds, startOfDay, timeText, viewDays } from './calendarTime';

interface Props {
  /** The account whose server is asked. */
  accountId: string;
  ownEmail: string;
  /** Calendars whose events count as your own busy times. */
  ownCalendarIds: readonly string[];
  /** People invited (addresses). */
  attendees: readonly string[];
  /** The event's current start, and how long it is (seconds). */
  start: number;
  duration: number;
  /** Leave the event being edited out of your own busy times. */
  eventId?: string | undefined;
  /** Unix seconds: slots before it aren't suggested. */
  now: number;
  onClose: () => void;
  onPick: (start: number, end: number) => void;
}

/** Working hours shown: 07:00 to 19:00, in half hours. */
const FIRST = 7 * 60;
const LAST = 19 * 60;
const STEP = 30;
const SLOTS = (LAST - FIRST) / STEP;
/** Suggestions are inside 08:00–18:00. */
const WORK_START = 8 * 60;
const WORK_END = 18 * 60;

type Range = 'day' | 'week';

interface Row {
  key: string;
  label: string;
  busy: FreeBusy['busy'];
  unknown: boolean;
}

const STATE_TEXT: Record<ShowAs | 'free' | 'unknown', string> = {
  busy: 'Busy',
  tentative: 'Tentative',
  away: 'Away',
  free: 'Free',
  unknown: 'Unknown',
};

/** What a person is doing in [from, to): the strongest of their overlapping blocks. */
function stateAt(row: Row, from: number, to: number): ShowAs | 'unknown' {
  if (row.unknown) return 'unknown';
  let state: ShowAs = 'free';
  const rank: Record<ShowAs, number> = { free: 0, tentative: 1, busy: 2, away: 3 };
  for (const b of row.busy) {
    if (b.start < to && b.end > from && rank[b.showAs] > rank[state]) state = b.showAs;
  }
  return state;
}

/**
 * Find a Time: when everyone invited (and you) is busy over a day or a work
 * week, with free times suggested. Picking a slot sets the event's time.
 */
export function FindTime({
  accountId,
  ownEmail,
  ownCalendarIds,
  attendees,
  start,
  duration,
  eventId,
  now,
  onClose,
  onPick,
}: Props) {
  const api = useMailApi();
  const [day, setDay] = useState(() => startOfDay(new Date(start * 1000)));
  const [range, setRange] = useState<Range>('day');
  const [rows, setRows] = useState<Row[] | null>(null);
  const [error, setError] = useState<AppError | null>(null);
  const days = useMemo(() => (range === 'day' ? [day] : viewDays('workweek', day)), [range, day]);
  const from = seconds(days[0] ?? day);
  const to = seconds(addDays(days[days.length - 1] ?? day, 1));
  const length = Math.max(STEP * 60, duration);
  const people = attendees.join('\n');
  const calendarKey = ownCalendarIds.join('\n');

  useEffect(() => {
    let live = true;
    const emails = people ? people.split('\n') : [];
    const calendars = new Set(calendarKey.split('\n'));
    Promise.all([
      emails.length > 0 ? api.getFreeBusy(accountId, emails, from, to) : Promise.resolve([]),
      api.listEvents(from, to),
    ]).then(
      ([others, mine]) => {
        if (!live) return;
        const own: Row = {
          key: 'you',
          label: `You (${ownEmail})`,
          unknown: false,
          busy: mine
            .filter((e) => calendars.has(e.calendarId) && e.id !== eventId && e.showAs !== 'free' && !e.allDay)
            .map((e) => ({ start: e.start, end: e.end, showAs: e.showAs })),
        };
        setRows([own, ...others.map((f) => ({ key: f.email.toLowerCase(), label: f.email, busy: f.busy, unknown: f.unknown }))]);
        setError(null);
      },
      (e: unknown) => {
        if (live) setError(toAppError(e));
      },
    );
    return () => {
      live = false;
    };
  }, [api, accountId, ownEmail, calendarKey, people, from, to, eventId]);

  const suggestions = useMemo(() => {
    if (!rows) return [];
    const known = rows.filter((r) => !r.unknown);
    const out: { start: number; end: number }[] = [];
    for (const d of days) {
      for (let m = WORK_START; m + length / 60 <= WORK_END && out.length < 6; m += STEP) {
        const s = atMinutes(d, m);
        if (s < now) continue;
        if (known.every((r) => stateAt(r, s, s + length) === 'free')) out.push({ start: s, end: s + length });
      }
    }
    return out;
  }, [rows, days, length, now]);

  const step = (n: number) => {
    setDay((d) => addDays(d, range === 'day' ? n : n * 7));
  };
  const chosenStart = start;
  const chosenEnd = start + length;

  return (
    <Modal title="Find a time" description="When everyone is free. Pick a time to use it." onClose={onClose} size="large">
      <div className="find-time">
        <div className="find-time__bar">
          <button
            type="button"
            className="icon-btn"
            aria-label={range === 'day' ? 'Previous day' : 'Previous week'}
            onClick={() => {
              step(-1);
            }}
          >
            <Icon name="left" />
          </button>
          <span className="find-time__range" aria-live="polite">
            {range === 'day' ? dayText(day) : `${dayText(days[0] ?? day)} – ${dayText(days[days.length - 1] ?? day)}`}
          </span>
          <button
            type="button"
            className="icon-btn"
            aria-label={range === 'day' ? 'Next day' : 'Next week'}
            onClick={() => {
              step(1);
            }}
          >
            <Icon name="right" />
          </button>
          <div className="segmented" role="group" aria-label="Show">
            {(['day', 'week'] as const).map((r) => (
              <button
                key={r}
                type="button"
                className={`segmented__item${range === r ? ' segmented__item--on' : ''}`}
                aria-pressed={range === r}
                onClick={() => {
                  setRange(r);
                }}
              >
                {r === 'day' ? 'Day' : 'Work week'}
              </button>
            ))}
          </div>
        </div>

        {error && (
          <p className="form-error" role="alert">
            {error.message}
          </p>
        )}
        {!rows && !error && (
          <p className="muted" role="status">
            Checking when people are free…
          </p>
        )}

        {rows && (
          <>
            <div className="find-time__scroll">
              <table className="find-time__grid" aria-label="Free and busy times">
                <thead>
                  <tr>
                    <th scope="col" className="find-time__who">
                      <span className="visually-hidden">Person</span>
                    </th>
                    {days.map((d) =>
                      Array.from({ length: SLOTS }, (_, i) => {
                        const m = FIRST + i * STEP;
                        const s = atMinutes(d, m);
                        const chosen = s >= chosenStart && s < chosenEnd;
                        return (
                          <th key={`${String(d.getTime())}-${String(i)}`} scope="col" className="find-time__time">
                            <button
                              type="button"
                              className={`find-time__slot${chosen ? ' find-time__slot--chosen' : ''}${i === 0 ? ' find-time__slot--day' : ''}`}
                              aria-label={`${dayText(d)}, ${hhmm(m)}`}
                              aria-pressed={chosen}
                              onClick={() => {
                                onPick(s, s + length);
                              }}
                            >
                              {i === 0 && days.length > 1 ? dayText(d) : m % 60 === 0 ? hhmm(m) : ''}
                            </button>
                          </th>
                        );
                      }),
                    )}
                  </tr>
                </thead>
                <tbody>
                  {rows.map((row) => (
                    <tr key={row.key}>
                      <th scope="row" className="find-time__who">
                        {row.label}
                        {row.unknown && <span className="muted small"> · no free/busy information</span>}
                      </th>
                      {days.map((d) =>
                        Array.from({ length: SLOTS }, (_, i) => {
                          const s = atMinutes(d, FIRST + i * STEP);
                          const state = stateAt(row, s, s + STEP * 60);
                          return (
                            <td
                              key={`${String(d.getTime())}-${String(i)}`}
                              className={`find-time__cell find-time__cell--${state}${s >= chosenStart && s < chosenEnd ? ' find-time__cell--chosen' : ''}`}
                              data-state={state}
                              data-time={`${dayText(d)}, ${hhmm(FIRST + i * STEP)}`}
                              title={STATE_TEXT[state]}
                            />
                          );
                        }),
                      )}
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
            <ul className="find-time__legend" aria-label="Key">
              <li>
                <span className="find-time__swatch find-time__cell--busy" aria-hidden="true" /> Busy
              </li>
              <li>
                <span className="find-time__swatch find-time__cell--tentative" aria-hidden="true" /> Tentative
              </li>
              <li>
                <span className="find-time__swatch find-time__cell--away" aria-hidden="true" /> Away
              </li>
              <li>
                <span className="find-time__swatch find-time__cell--unknown" aria-hidden="true" /> No information
              </li>
            </ul>
            <h3 className="find-time__subhead">Suggested times</h3>
            {suggestions.length === 0 ? (
              <p className="muted">No time when everyone is free. Try another day.</p>
            ) : (
              <ul className="find-time__suggestions" aria-label="Suggested times">
                {suggestions.map((s) => (
                  <li key={s.start}>
                    <button
                      type="button"
                      className="btn btn--secondary btn--small"
                      onClick={() => {
                        onPick(s.start, s.end);
                      }}
                    >
                      {`${dayText(new Date(s.start * 1000))}, ${timeText(s.start)} – ${timeText(s.end)}`}
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </>
        )}
        <div className="modal__actions">
          <button type="button" className="btn btn--secondary" onClick={onClose}>
            Close
          </button>
        </div>
      </div>
    </Modal>
  );
}
