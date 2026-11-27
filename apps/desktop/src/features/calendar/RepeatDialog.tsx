import { useId, useState, type SyntheticEvent } from 'react';

import type { RepeatRule } from '../../api/types';
import { Modal } from '../../ui/Modal';
import { dayKey, firstDayOfWeek, fromDayKey, utcMidnight } from './calendarTime';
import { DAY_CODES, dayName, ordinal, ruleText, startingRule, weekdayPosition } from './repeatRule';

interface Props {
  /** The event's first date. */
  start: Date;
  /** The rule so far, or null for a new one. */
  initial: RepeatRule | null;
  onClose: () => void;
  onDone: (rule: RepeatRule) => void;
}

type End = 'never' | 'count' | 'until';

const UNIT: Record<RepeatRule['freq'], [string, string]> = {
  daily: ['day', 'days'],
  weekly: ['week', 'weeks'],
  monthly: ['month', 'months'],
  yearly: ['year', 'years'],
};

const monthName = (month: number) => new Intl.DateTimeFormat(undefined, { month: 'long' }).format(new Date(2024, month - 1, 1));

/**
 * Custom repeat: every N days, weeks (on chosen weekdays), months (on a date
 * or "the second Tuesday") or years, ending never, after N times or on a day.
 */
export function RepeatDialog({ start, initial, onClose, onDone }: Props) {
  const id = useId();
  const [rule, setRule] = useState<RepeatRule>(() => initial ?? startingRule('weekly', start));
  const [end, setEnd] = useState<End>(() => (initial?.count != null ? 'count' : initial?.until != null ? 'until' : 'never'));
  const [count, setCount] = useState(String(initial?.count ?? 10));
  const [until, setUntil] = useState(() => {
    if (initial?.until != null) {
      const d = new Date(initial.until * 1000);
      return dayKey(new Date(d.getUTCFullYear(), d.getUTCMonth(), d.getUTCDate()));
    }
    return dayKey(new Date(start.getFullYear(), start.getMonth() + 3, start.getDate()));
  });
  const [error, setError] = useState<string | null>(null);

  const startCode = DAY_CODES[start.getDay()] ?? 'MO';
  const position = weekdayPosition(start);
  const byPosition = rule.setPos !== null;
  const first = firstDayOfWeek();
  const week = Array.from({ length: 7 }, (_, i) => DAY_CODES[(first + i) % 7] ?? 'MO');

  const setFreq = (freq: RepeatRule['freq']) => {
    setRule((r) => ({ ...startingRule(freq, start), interval: r.interval }));
  };

  const finished = (): RepeatRule | string => {
    const n = Number(count);
    const base: RepeatRule = {
      ...rule,
      interval: Math.max(1, Math.round(rule.interval) || 1),
      count: null,
      until: null,
    };
    if (base.freq === 'weekly' && base.byDay.length === 0) return 'Pick at least one day of the week.';
    if (end === 'count') {
      if (!Number.isInteger(n) || n < 1 || n > 999) return 'Enter how many times, from 1 to 999.';
      base.count = n;
    }
    if (end === 'until') {
      if (!until) return 'Choose the last day.';
      if (fromDayKey(until) < fromDayKey(dayKey(start))) return 'The last day is before the event starts.';
      base.until = utcMidnight(until);
    }
    return base;
  };

  const submit = (e: SyntheticEvent) => {
    e.preventDefault();
    const next = finished();
    if (typeof next === 'string') {
      setError(next);
      return;
    }
    onDone(next);
  };

  const preview = finished();
  const [one, many] = UNIT[rule.freq];

  return (
    <Modal title="Repeat" onClose={onClose} size="medium">
      <form className="repeat-dialog" aria-label="Custom repeat" noValidate onSubmit={submit}>
        <div className="repeat-dialog__every">
          <label className="form-field">
            <span className="form-field__label">Repeat every</span>
            <input
              className="field repeat-dialog__number"
              type="number"
              min={1}
              max={99}
              value={rule.interval}
              onChange={(e) => {
                setRule((r) => ({ ...r, interval: Number(e.target.value) }));
              }}
            />
          </label>
          <label className="form-field">
            <span className="form-field__label visually-hidden">Unit</span>
            <select
              className="field"
              aria-label="Unit"
              value={rule.freq}
              onChange={(e) => {
                setFreq(e.target.value as RepeatRule['freq']);
              }}
            >
              {(Object.keys(UNIT) as RepeatRule['freq'][]).map((f) => (
                <option key={f} value={f}>
                  {rule.interval === 1 ? UNIT[f][0] : UNIT[f][1]}
                </option>
              ))}
            </select>
          </label>
        </div>

        {rule.freq === 'weekly' && (
          <fieldset className="repeat-dialog__days">
            <legend className="form-field__label">On</legend>
            {week.map((code) => {
              const on = rule.byDay.includes(code);
              return (
                <button
                  key={code}
                  type="button"
                  className={`day-toggle${on ? ' day-toggle--on' : ''}`}
                  aria-pressed={on}
                  aria-label={dayName(code)}
                  onClick={() => {
                    setRule((r) => ({ ...r, byDay: on ? r.byDay.filter((c) => c !== code) : [...r.byDay, code] }));
                  }}
                >
                  {dayName(code).slice(0, 2)}
                </button>
              );
            })}
          </fieldset>
        )}

        {(rule.freq === 'monthly' || rule.freq === 'yearly') && (
          <fieldset className="repeat-dialog__on">
            <legend className="form-field__label">On</legend>
            <label className="choice">
              <input
                type="radio"
                name={`${id}-on`}
                checked={!byPosition}
                onChange={() => {
                  setRule((r) => ({ ...r, setPos: null, byDay: [], monthDay: start.getDate() }));
                }}
              />
              {rule.freq === 'monthly'
                ? `Day ${String(start.getDate())}`
                : `${String(start.getDate())} ${monthName(start.getMonth() + 1)}`}
            </label>
            <label className="choice">
              <input
                type="radio"
                name={`${id}-on`}
                checked={byPosition}
                onChange={() => {
                  setRule((r) => ({ ...r, setPos: position, byDay: [startCode], monthDay: null }));
                }}
              />
              {`The ${ordinal(position)} ${dayName(startCode)}${rule.freq === 'yearly' ? ` of ${monthName(start.getMonth() + 1)}` : ''}`}
            </label>
          </fieldset>
        )}

        <fieldset className="repeat-dialog__end">
          <legend className="form-field__label">Ends</legend>
          <label className="choice">
            <input
              type="radio"
              name={`${id}-end`}
              checked={end === 'never'}
              onChange={() => {
                setEnd('never');
              }}
            />
            Never
          </label>
          <div className="repeat-dialog__end-row">
            <label className="choice">
              <input
                type="radio"
                name={`${id}-end`}
                checked={end === 'count'}
                onChange={() => {
                  setEnd('count');
                }}
              />
              After
            </label>
            <input
              className="field repeat-dialog__number"
              type="number"
              min={1}
              max={999}
              aria-label="Number of times"
              value={count}
              disabled={end !== 'count'}
              onChange={(e) => {
                setCount(e.target.value);
              }}
            />
            <span>times</span>
          </div>
          <div className="repeat-dialog__end-row">
            <label className="choice">
              <input
                type="radio"
                name={`${id}-end`}
                checked={end === 'until'}
                onChange={() => {
                  setEnd('until');
                }}
              />
              On
            </label>
            <input
              className="field"
              type="date"
              aria-label="Last day"
              value={until}
              min={dayKey(start)}
              disabled={end !== 'until'}
              onChange={(e) => {
                setUntil(e.target.value);
              }}
            />
          </div>
        </fieldset>

        <p className="form-hint" aria-live="polite">
          {typeof preview === 'string' ? `Every ${rule.interval === 1 ? one : `${String(rule.interval)} ${many}`}` : ruleText(preview, start)}
        </p>
        {error && (
          <p className="form-error" role="alert">
            {error}
          </p>
        )}
        <div className="modal__actions">
          <button type="button" className="btn btn--secondary" onClick={onClose}>
            Cancel
          </button>
          <button type="submit" className="btn btn--primary">
            Done
          </button>
        </div>
      </form>
    </Modal>
  );
}
