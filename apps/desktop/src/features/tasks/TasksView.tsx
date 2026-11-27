import { useEffect, useId, useMemo, useState, type SyntheticEvent } from 'react';

import { useMailApi } from '../../api/MailApiContext';
import { toAppError } from '../../api/errors';
import type { Account, AppError, Category, Importance, Task, TaskInput, TaskList } from '../../api/types';
import { Icon } from '../../ui/Icon';
import { ConfirmDialog } from '../../ui/Modal';
import { ConnectCards, useConnectable } from '../connect/ConnectCard';
import { addDays, dayKey, dayText, startOfDay, utcMidnight } from '../calendar/calendarTime';

interface Props {
  accounts: readonly Account[];
  phone: boolean;
  /** Unix seconds. */
  now: number;
  categories: readonly Category[];
  /** Open this task (from a reminder). */
  focusTaskId?: string | null;
  onClose: () => void;
  onOpenMessage: (messageId: string) => void;
}

export type TaskFilter = 'today' | 'week' | 'all' | 'completed';
type Show = { kind: 'filter'; filter: TaskFilter } | { kind: 'list'; id: string };

const FILTERS: { filter: TaskFilter; label: string }[] = [
  { filter: 'today', label: 'Today' },
  { filter: 'week', label: 'Next 7 days' },
  { filter: 'all', label: 'All' },
  { filter: 'completed', label: 'Completed' },
];

const IMPORTANCE: { value: Importance; label: string }[] = [
  { value: 'high', label: 'High' },
  { value: 'normal', label: 'Normal' },
  { value: 'low', label: 'Low' },
];

/** A due date (midnight UTC) as the local calendar day. */
export function dueDay(due: number): Date {
  const d = new Date(due * 1000);
  return new Date(d.getUTCFullYear(), d.getUTCMonth(), d.getUTCDate());
}

/** Whether a task's due day is before today. */
export function isOverdue(task: Task, now: number): boolean {
  return !task.completed && task.due !== null && dueDay(task.due) < startOfDay(new Date(now * 1000));
}

/** "Today", "Tomorrow", "Overdue · Tue 10 Nov", "Mon 16 Nov". */
export function dueText(task: Task, now: number): string {
  if (task.due === null) return '';
  const day = dueDay(task.due);
  const today = startOfDay(new Date(now * 1000));
  if (dayKey(day) === dayKey(today)) return 'Today';
  if (dayKey(day) === dayKey(addDays(today, 1))) return 'Tomorrow';
  return isOverdue(task, now) ? `Overdue · ${dayText(day)}` : dayText(day);
}

/** Which tasks a filter shows. */
export function matchesFilter(task: Task, filter: TaskFilter, now: number): boolean {
  if (filter === 'completed') return task.completed;
  if (task.completed) return false;
  if (filter === 'all') return true;
  if (task.due === null) return false;
  const today = startOfDay(new Date(now * 1000));
  return dueDay(task.due) <= addDays(today, filter === 'today' ? 0 : 7);
}

export function inputOf(t: Task): TaskInput {
  return {
    id: t.id,
    listId: t.listId,
    title: t.title,
    notes: t.notes,
    due: t.due,
    reminder: t.reminder,
    importance: t.importance,
    completed: t.completed,
    messageId: t.messageId,
    categories: t.categories,
  };
}

/** The list new tasks go to: the first one on this device, else the first. */
export function defaultList(lists: readonly TaskList[]): TaskList | undefined {
  return lists.find((l) => l.provider === 'device') ?? lists[0];
}

/** `<input type="datetime-local">` value for Unix seconds. */
function localValue(secs: number | null): string {
  if (secs === null) return '';
  const d = new Date(secs * 1000);
  const pad = (n: number) => String(n).padStart(2, '0');
  return `${dayKey(d)}T${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

function fromLocalValue(value: string): number | null {
  if (!value) return null;
  const t = new Date(value).getTime();
  return Number.isNaN(t) ? null : Math.floor(t / 1000);
}

/**
 * To-do lists: on this device and on each account (CalDAV, Google Tasks,
 * Microsoft To Do). Filters for today, the next week, everything and what's
 * done; add a task in one line, tick it off, and edit its details.
 */
export function TasksView({ accounts, phone, now, categories, focusTaskId, onClose, onOpenMessage }: Props) {
  const api = useMailApi();
  const id = useId();
  const [lists, setLists] = useState<TaskList[]>([]);
  const [tasks, setTasks] = useState<Task[] | null>(null);
  const [show, setShow] = useState<Show>({ kind: 'filter', filter: 'all' });
  const [revision, setRevision] = useState(0);
  const [openId, setOpenId] = useState<string | null>(null);
  const [error, setError] = useState<AppError | null>(null);
  const [syncing, setSyncing] = useState(true);
  const [adding, setAdding] = useState('');
  const statuses = useConnectable();

  const [seenFocus, setSeenFocus] = useState<string | null>(null);
  if (focusTaskId && focusTaskId !== seenFocus) {
    setSeenFocus(focusTaskId);
    setShow({ kind: 'filter', filter: 'all' });
    setOpenId(focusTaskId);
  }

  useEffect(() => {
    let live = true;
    const listId = show.kind === 'list' ? show.id : null;
    const includeCompleted = show.kind === 'filter' && show.filter === 'completed';
    Promise.all([api.listTaskLists(), api.listTasks(listId, includeCompleted || openId !== null)]).then(
      ([nextLists, nextTasks]) => {
        if (!live) return;
        setLists(nextLists);
        setTasks(nextTasks);
      },
      (e: unknown) => {
        if (live) setError(toAppError(e));
      },
    );
    return () => {
      live = false;
    };
  }, [api, show, revision, openId]);

  useEffect(() => {
    const off = api.on('mail://tasks-changed', () => {
      setRevision((r) => r + 1);
    });
    api.syncTasks().then(
      () => {
        setSyncing(false);
      },
      () => {
        setSyncing(false);
      },
    );
    return off;
  }, [api]);

  const shown = useMemo(() => {
    const all = tasks ?? [];
    if (show.kind === 'list') return all.filter((t) => !t.completed && t.listId === show.id);
    return all.filter((t) => matchesFilter(t, show.filter, now));
  }, [tasks, show, now]);
  const opened = (tasks ?? []).find((t) => t.id === openId) ?? null;
  const listName = (listId: string) => lists.find((l) => l.id === listId)?.name ?? '';
  const heading = show.kind === 'list' ? listName(show.id) : (FILTERS.find((f) => f.filter === show.filter)?.label ?? '');
  const target = show.kind === 'list' ? lists.find((l) => l.id === show.id) : defaultList(lists);

  const saveTask = (input: TaskInput, then?: (t: Task) => void) => {
    setError(null);
    api.saveTask(input).then(
      (saved) => {
        setRevision((r) => r + 1);
        then?.(saved);
      },
      (e: unknown) => {
        setError(toAppError(e));
      },
    );
  };

  const add = (e: SyntheticEvent) => {
    e.preventDefault();
    const title = adding.trim();
    if (!title || !target) return;
    const today = dayKey(new Date(now * 1000));
    saveTask(
      {
        id: null,
        listId: target.id,
        title,
        notes: '',
        due: show.kind === 'filter' && (show.filter === 'today' || show.filter === 'week') ? utcMidnight(today) : null,
        reminder: null,
        importance: 'normal',
        completed: false,
        messageId: null,
        categories: [],
      },
      () => {
        setAdding('');
      },
    );
  };

  const choose = (next: Show) => {
    setShow(next);
    setOpenId(null);
  };

  const showList = !phone || opened === null;
  const showDetail = opened !== null;

  const listsByAccount = [
    { label: 'On this device', items: lists.filter((l) => l.accountId === null) },
    ...accounts.map((a) => ({ label: a.displayName || a.email, items: lists.filter((l) => l.accountId === a.id) })),
  ].filter((g) => g.items.length > 0);

  return (
    <section className="tasks" aria-labelledby={`${id}-title`}>
      <header className="tasks__head">
        <h1 id={`${id}-title`} className="tasks__title">
          Tasks
        </h1>
        <button
          type="button"
          className="icon-btn"
          aria-label="Sync tasks"
          title="Sync tasks"
          disabled={syncing}
          onClick={() => {
            setSyncing(true);
            api.syncTasks().then(
              () => {
                setSyncing(false);
                setRevision((r) => r + 1);
              },
              (e: unknown) => {
                setSyncing(false);
                setError(toAppError(e));
              },
            );
          }}
        >
          <Icon name="sync" />
        </button>
        <button type="button" className="icon-btn" aria-label="Close tasks" onClick={onClose}>
          <Icon name="close" />
        </button>
      </header>
      <ConnectCards
        accounts={accounts}
        statuses={statuses}
        onConnected={() => {
          setRevision((r) => r + 1);
        }}
      />
      <div className="tasks__body">
        {!phone && (
          <nav className="tasks__nav" aria-label="Task lists">
            <ul className="tasks__filters">
              {FILTERS.map((f) => {
                const on = show.kind === 'filter' && show.filter === f.filter;
                return (
                  <li key={f.filter}>
                    <button
                      type="button"
                      className={`nav__item${on ? ' nav__item--active' : ''}`}
                      aria-current={on ? 'page' : undefined}
                      onClick={() => {
                        choose({ kind: 'filter', filter: f.filter });
                      }}
                    >
                      <Icon name={f.filter === 'completed' ? 'check' : f.filter === 'all' ? 'all' : 'calendar'} />
                      {f.label}
                    </button>
                  </li>
                );
              })}
            </ul>
            {listsByAccount.map((g) => (
              <div key={g.label} className="tasks__group">
                <h2 className="tasks__group-title">{g.label}</h2>
                <ul className="tasks__filters">
                  {g.items.map((l) => {
                    const on = show.kind === 'list' && show.id === l.id;
                    return (
                      <li key={l.id}>
                        <button
                          type="button"
                          className={`nav__item${on ? ' nav__item--active' : ''}`}
                          aria-current={on ? 'page' : undefined}
                          onClick={() => {
                            choose({ kind: 'list', id: l.id });
                          }}
                        >
                          <Icon name="menu" />
                          {l.name}
                        </button>
                      </li>
                    );
                  })}
                </ul>
              </div>
            ))}
          </nav>
        )}
        {showList && (
          <div className="tasks__main">
            {phone && (
              <label className="form-field tasks__picker">
                <span className="visually-hidden">Show</span>
                <select
                  className="field"
                  aria-label="Show"
                  value={show.kind === 'list' ? `list:${show.id}` : show.filter}
                  onChange={(e) => {
                    const v = e.target.value;
                    choose(v.startsWith('list:') ? { kind: 'list', id: v.slice(5) } : { kind: 'filter', filter: v as TaskFilter });
                  }}
                >
                  {FILTERS.map((f) => (
                    <option key={f.filter} value={f.filter}>
                      {f.label}
                    </option>
                  ))}
                  {lists.map((l) => (
                    <option key={l.id} value={`list:${l.id}`}>
                      {l.name}
                    </option>
                  ))}
                </select>
              </label>
            )}
            {!phone && <h2 className="tasks__heading">{heading}</h2>}
            {error && (
              <p className="form-error" role="alert">
                {error.message}
              </p>
            )}
            {!(show.kind === 'filter' && show.filter === 'completed') && target && (
              <form className="tasks__add" aria-label="Add a task" onSubmit={add}>
                <Icon name="plus" />
                <input
                  className="field"
                  aria-label="Add a task"
                  placeholder={`Add a task to ${target.name}`}
                  value={adding}
                  maxLength={500}
                  onChange={(e) => {
                    setAdding(e.target.value);
                  }}
                />
              </form>
            )}
            {tasks === null ? (
              <p className="muted" role="status">
                Loading tasks…
              </p>
            ) : shown.length === 0 ? (
              <p className="muted tasks__empty">
                {show.kind === 'filter' && show.filter === 'completed' ? 'Nothing completed yet.' : 'Nothing to do here.'}
              </p>
            ) : (
              <ul className="tasks__list" aria-label={`${heading} tasks`}>
                {shown.map((t) => (
                  <li key={t.id} className={`task-row${t.id === openId ? ' task-row--active' : ''}${t.completed ? ' task-row--done' : ''}`}>
                    <input
                      type="checkbox"
                      className="task-row__check"
                      checked={t.completed}
                      aria-label={t.completed ? `Mark ${t.title} as not done` : `Complete ${t.title}`}
                      onChange={(e) => {
                        saveTask({ ...inputOf(t), completed: e.target.checked });
                      }}
                    />
                    <button
                      type="button"
                      className="task-row__main"
                      aria-current={t.id === openId ? 'true' : undefined}
                      onClick={() => {
                        setOpenId(t.id);
                      }}
                    >
                      <span className="task-row__title">{t.title}</span>
                      <span className="task-row__meta">
                        {t.due !== null && (
                          <span className={`badge badge--due${isOverdue(t, now) ? ' badge--overdue' : ''}`}>{dueText(t, now)}</span>
                        )}
                        {show.kind === 'filter' && <span className="muted small">{listName(t.listId)}</span>}
                        {t.messageId && (
                          <span className="muted small" title="From a message">
                            <Icon name="mail" size={14} />
                            <span className="visually-hidden">From a message</span>
                          </span>
                        )}
                        {t.categories.map((c) => (
                          <span
                            key={c}
                            className="category-chip"
                            style={{ ['--category' as string]: categories.find((x) => x.name === c)?.color ?? 'var(--muted)' }}
                          >
                            {c}
                          </span>
                        ))}
                      </span>
                    </button>
                    {t.importance === 'high' && (
                      <span className="task-row__important" title="High importance">
                        <Icon name="alert" size={16} />
                        <span className="visually-hidden">High importance</span>
                      </span>
                    )}
                  </li>
                ))}
              </ul>
            )}
          </div>
        )}
        {showDetail && (
          <aside className="tasks__detail" aria-label="Task details">
            <TaskEditor
              key={opened.id}
              task={opened}
              lists={lists}
              categories={categories}
              phone={phone}
              onClose={() => {
                setOpenId(null);
              }}
              onSave={(input) => {
                saveTask(input, () => {
                  setOpenId(null);
                });
              }}
              onDelete={() => {
                api.deleteTask(opened.id).then(
                  () => {
                    setOpenId(null);
                    setRevision((r) => r + 1);
                  },
                  (e: unknown) => {
                    setError(toAppError(e));
                  },
                );
              }}
              onOpenMessage={onOpenMessage}
            />
          </aside>
        )}
      </div>
    </section>
  );
}

function TaskEditor({
  task,
  lists,
  categories,
  phone,
  onClose,
  onSave,
  onDelete,
  onOpenMessage,
}: {
  task: Task;
  lists: readonly TaskList[];
  categories: readonly Category[];
  phone: boolean;
  onClose: () => void;
  onSave: (input: TaskInput) => void;
  onDelete: () => void;
  onOpenMessage: (messageId: string) => void;
}) {
  const [input, setInput] = useState<TaskInput>(() => inputOf(task));
  const [confirm, setConfirm] = useState(false);
  const set = <K extends keyof TaskInput>(key: K, value: TaskInput[K]) => {
    setInput((i) => ({ ...i, [key]: value }));
  };
  const submit = (e: SyntheticEvent) => {
    e.preventDefault();
    onSave(input);
  };
  const due = input.due === null ? '' : dayKey(dueDay(input.due));
  const names = [...new Set([...categories.map((c) => c.name), ...input.categories])];
  return (
    <form className="task-editor" aria-label="Task" noValidate onSubmit={submit}>
      {phone && (
        <button type="button" className="phone-back" onClick={onClose}>
          <Icon name="back" />
          Back
        </button>
      )}
      <label className="form-field">
        <span className="form-field__label">Title</span>
        <input
          className="field"
          value={input.title}
          maxLength={500}
          onChange={(e) => {
            set('title', e.target.value);
          }}
        />
      </label>
      <label className="form-field">
        <span className="form-field__label">Notes</span>
        <textarea
          className="field task-editor__notes"
          rows={4}
          value={input.notes}
          onChange={(e) => {
            set('notes', e.target.value);
          }}
        />
      </label>
      <div className="task-editor__row">
        <label className="form-field">
          <span className="form-field__label">Due</span>
          <input
            className="field"
            type="date"
            value={due}
            onChange={(e) => {
              set('due', e.target.value ? utcMidnight(e.target.value) : null);
            }}
          />
        </label>
        <label className="form-field">
          <span className="form-field__label">Remind me</span>
          <input
            className="field"
            type="datetime-local"
            value={localValue(input.reminder)}
            onChange={(e) => {
              set('reminder', fromLocalValue(e.target.value));
            }}
          />
        </label>
      </div>
      <div className="task-editor__row">
        <label className="form-field">
          <span className="form-field__label">Importance</span>
          <select
            className="field"
            value={input.importance}
            onChange={(e) => {
              set('importance', e.target.value as Importance);
            }}
          >
            {IMPORTANCE.map((i) => (
              <option key={i.value} value={i.value}>
                {i.label}
              </option>
            ))}
          </select>
        </label>
        <label className="form-field">
          <span className="form-field__label">List</span>
          <select
            className="field"
            value={input.listId}
            onChange={(e) => {
              set('listId', e.target.value);
            }}
          >
            {lists.map((l) => (
              <option key={l.id} value={l.id}>
                {l.name}
              </option>
            ))}
          </select>
        </label>
      </div>
      {names.length > 0 && (
        <fieldset className="task-editor__categories">
          <legend className="form-field__label">Categories</legend>
          {names.map((name) => (
            <label key={name} className="choice">
              <input
                type="checkbox"
                checked={input.categories.includes(name)}
                onChange={(e) => {
                  set('categories', e.target.checked ? [...input.categories, name] : input.categories.filter((c) => c !== name));
                }}
              />
              {name}
            </label>
          ))}
        </fieldset>
      )}
      <label className="choice">
        <input
          type="checkbox"
          checked={input.completed}
          onChange={(e) => {
            set('completed', e.target.checked);
          }}
        />
        Completed
      </label>
      {task.messageId && (
        <button
          type="button"
          className="btn btn--secondary btn--small task-editor__message"
          onClick={() => {
            if (task.messageId) onOpenMessage(task.messageId);
          }}
        >
          <Icon name="mail" />
          Open message
        </button>
      )}
      <div className="modal__actions">
        <button
          type="button"
          className="btn btn--link btn--danger-text"
          onClick={() => {
            setConfirm(true);
          }}
        >
          Delete
        </button>
        <button type="button" className="btn btn--secondary" onClick={onClose}>
          Cancel
        </button>
        <button type="submit" className="btn btn--primary">
          Save task
        </button>
      </div>
      {confirm && (
        <ConfirmDialog
          title={`Delete “${task.title}”?`}
          description="The task is deleted here and from its list on the server."
          confirmLabel="Delete task"
          danger
          onCancel={() => {
            setConfirm(false);
          }}
          onConfirm={() => {
            setConfirm(false);
            onDelete();
          }}
        />
      )}
    </form>
  );
}
