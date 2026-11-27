import { useState } from 'react';

import type { Account, CalendarInfo } from '../../api/types';
import { Icon } from '../../ui/Icon';
import { addMonths, cssColor, dayKey, dayText, monthGrid, weekdayNames, type DayKey } from './calendarTime';

const MONTH = new Intl.DateTimeFormat(undefined, { month: 'long', year: 'numeric' });

/** A small month for jumping to a date. */
export function MiniMonth({
  selected,
  todayKey,
  onPick,
}: {
  selected: Date;
  todayKey: DayKey;
  onPick: (day: Date) => void;
}) {
  const [month, setMonth] = useState(() => new Date(selected.getFullYear(), selected.getMonth(), 1));
  // Follow the views when they move to another month.
  const [seen, setSeen] = useState(dayKey(selected));
  if (seen !== dayKey(selected)) {
    setSeen(dayKey(selected));
    setMonth(new Date(selected.getFullYear(), selected.getMonth(), 1));
  }
  const grid = monthGrid(month);
  const selectedKey = dayKey(selected);
  return (
    <div className="mini-month">
      <div className="mini-month__head">
        <span className="mini-month__title">{MONTH.format(month)}</span>
        <button
          type="button"
          className="icon-btn icon-btn--small"
          aria-label="Earlier month"
          onClick={() => {
            setMonth((m) => addMonths(m, -1));
          }}
        >
          <Icon name="left" size={16} />
        </button>
        <button
          type="button"
          className="icon-btn icon-btn--small"
          aria-label="Later month"
          onClick={() => {
            setMonth((m) => addMonths(m, 1));
          }}
        >
          <Icon name="right" size={16} />
        </button>
      </div>
      <table className="mini-month__grid" aria-label={`Go to a date in ${MONTH.format(month)}`}>
        <thead>
          <tr>
            {weekdayNames(undefined, 'narrow').map((w, i) => (
              <th key={i} scope="col">
                {w}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {Array.from({ length: 6 }, (_, week) => (
            <tr key={week}>
              {grid.slice(week * 7, week * 7 + 7).map((d) => {
                const key = dayKey(d);
                return (
                  <td key={key}>
                    <button
                      type="button"
                      className={[
                        'mini-month__day',
                        d.getMonth() === month.getMonth() ? '' : 'mini-month__day--other',
                        key === todayKey ? 'mini-month__day--today' : '',
                        key === selectedKey ? 'mini-month__day--on' : '',
                      ]
                        .filter(Boolean)
                        .join(' ')}
                      aria-label={`Go to ${dayText(d)}`}
                      aria-current={key === todayKey ? 'date' : undefined}
                      onClick={() => {
                        onPick(d);
                      }}
                    >
                      {d.getDate()}
                    </button>
                  </td>
                );
              })}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

/** Each account's calendars with a colour and a tick to show or hide them. */
export function CalendarList({
  accounts,
  calendars,
  onToggle,
}: {
  accounts: readonly Account[];
  calendars: readonly CalendarInfo[];
  onToggle: (calendar: CalendarInfo, visible: boolean) => void;
}) {
  const groups = accounts
    .map((a) => ({ account: a, list: calendars.filter((c) => c.accountId === a.id) }))
    .filter((g) => g.list.length > 0);
  if (groups.length === 0) return null;
  return (
    <nav className="cal-list" aria-label="Calendars">
      {groups.map(({ account, list }) => (
        <div key={account.id} className="cal-list__group">
          <h3 className="cal-list__account" title={account.email}>
            {account.displayName || account.email}
          </h3>
          <ul className="cal-list__items">
            {list.map((c) => (
              <li key={c.id}>
                <label className="cal-list__item" style={{ ['--event-color' as string]: cssColor(c.color) }}>
                  <input
                    type="checkbox"
                    className="cal-list__check"
                    checked={c.visible}
                    onChange={(e) => {
                      onToggle(c, e.target.checked);
                    }}
                  />
                  <span className="cal-list__name">{c.name}</span>
                  {!c.writable && (
                    <span className="cal-list__lock" title="Read-only">
                      <Icon name="lock" size={12} />
                      <span className="visually-hidden"> (read-only)</span>
                    </span>
                  )}
                </label>
              </li>
            ))}
          </ul>
        </div>
      ))}
    </nav>
  );
}
