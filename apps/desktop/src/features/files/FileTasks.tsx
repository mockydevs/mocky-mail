import { createContext, use, useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';

import { useMailApi } from '../../api/MailApiContext';
import { toAppError } from '../../api/errors';
import type { AppError, ImportProgress, ImportReport } from '../../api/types';
import type { Notice } from '../../app/useToasts';
import { usePhoneBack } from '../../app/usePhone';
import { Modal } from '../../ui/Modal';
import { fileErrorCopy } from './fileErrors';

/** One import or export, started from a menu, Settings, or an opened file. */
export interface FileTask {
  /** The dialog's title while it runs, such as "Importing into Inbox". */
  title: string;
  /** What an item is, for the summary ("message", "event", "contact"). */
  noun: string;
  /** Calls the backend; a native picker comes first. null: the user cancelled. */
  start: () => Promise<ImportReport | string | null>;
  /** Long work (an Outlook file): the dialog says so and explains that closing it is fine. */
  long?: boolean;
  /** The summary's title, and the toast's when it finishes after the dialog was closed. */
  doneTitle?: string;
  /** The toast's title when it fails after the dialog was closed. */
  failedTitle?: string;
  /** After it worked: refresh what shows the new items. */
  onDone?: () => void;
}

export interface FileTasks {
  /** Start a task. Refused (with a toast) while another one is still running. */
  run: (task: FileTask) => void;
  /** A task is under way (its dialog may be closed). */
  busy: boolean;
}

type State =
  | { phase: 'running'; task: FileTask; hidden: boolean }
  | { phase: 'done'; task: FileTask; report: ImportReport }
  | { phase: 'failed'; task: FileTask; error: AppError };

const FileTasksContext = createContext<FileTasks | null>(null);

export function FileTasksProvider({ value, children }: { value: FileTasks; children: ReactNode }) {
  return <FileTasksContext value={value}>{children}</FileTasksContext>;
}

export function useFileTasks(): FileTasks {
  const tasks = use(FileTasksContext);
  if (!tasks) throw new Error('useFileTasks must be used inside FileTasksProvider');
  return tasks;
}

/** "1 message", "3 messages". */
export function countOf(n: number, noun: string): string {
  return `${String(n)} ${noun}${n === 1 ? '' : 's'}`;
}

const KINDS: readonly [keyof NonNullable<ImportReport['counts']>, string][] = [
  ['messages', 'message'],
  ['contacts', 'contact'],
  ['events', 'calendar event'],
  ['tasks', 'task'],
  ['notes', 'note'],
];

/** What came in, in one line: "4 messages, 1 contact and 2 tasks". */
export function reportLine(report: ImportReport, noun: string): string {
  const parts = report.counts
    ? KINDS.filter(([key]) => (report.counts?.[key] ?? 0) > 0).map(([key, word]) => countOf(report.counts?.[key] ?? 0, word))
    : [countOf(report.imported, noun)];
  if (parts.length === 0) return 'Nothing new was imported';
  const last = parts.pop() ?? '';
  return `${parts.length > 0 ? `${parts.join(', ')} and ${last}` : last} imported`;
}

/**
 * Runs imports and exports for the whole app: a progress dialog while
 * mail://import-progress events arrive (a bar, or a moving one while the
 * total is unknown), then a summary of what came in, or the error in plain
 * words. The dialog can be closed while the work goes on; a toast then says
 * when it has finished. An export ends with a toast saying where it went.
 */
export function useFileTaskRunner({
  onNotice,
  onOpenFolder,
  workOffline = false,
}: {
  onNotice: (notice: Notice) => void;
  /** "Open folder" in the summary. */
  onOpenFolder?: (folderId: string) => void;
  workOffline?: boolean;
}): { tasks: FileTasks; dialog: ReactNode } {
  const [state, setState] = useState<State | null>(null);
  const running = state?.phase === 'running';
  /** Kept in a ref so the event handler and finishing promise see the latest. */
  const runningRef = useRef(false);
  /** The running task's dialog was closed: its outcome comes as a toast. */
  const hiddenRef = useRef(false);
  const noticeRef = useRef(onNotice);
  const offlineRef = useRef(workOffline);
  useEffect(() => {
    noticeRef.current = onNotice;
    offlineRef.current = workOffline;
  });

  const run = useCallback(
    (task: FileTask) => {
      if (runningRef.current) {
        noticeRef.current({
          tone: 'info',
          title: 'Another import is still running',
          text: 'Wait for it to finish, then try again.',
        });
        return;
      }
      runningRef.current = true;
      hiddenRef.current = false;
      setState({ phase: 'running', task, hidden: false });
      task.start().then(
        (result) => {
          runningRef.current = false;
          if (result !== null) task.onDone?.();
          if (result === null) {
            setState(null);
          } else if (typeof result === 'string') {
            setState(null);
            noticeRef.current({ tone: 'info', title: 'Saved', text: result });
          } else if (!hiddenRef.current) {
            setState({ phase: 'done', task, report: result });
          } else {
            setState(null);
            noticeRef.current({
              tone: 'info',
              title: task.doneTitle ?? 'Import finished',
              text: `${reportLine(result, task.noun)}.${result.skipped > 0 ? ` ${countOf(result.skipped, 'item')} left out.` : ''}`,
              sticky: true,
              action: {
                label: 'Details',
                run: () => {
                  setState({ phase: 'done', task, report: result });
                },
              },
            });
          }
        },
        (e: unknown) => {
          runningRef.current = false;
          const error = toAppError(e);
          if (!hiddenRef.current) {
            setState({ phase: 'failed', task, error });
            return;
          }
          setState(null);
          const copy = fileErrorCopy(error, offlineRef.current);
          noticeRef.current({ tone: 'error', title: `${task.failedTitle ?? 'Import failed'}: ${copy.title}`, text: copy.text });
        },
      );
    },
    [],
  );

  const tasks = useMemo(() => ({ run, busy: running }), [run, running]);

  const close = () => {
    hiddenRef.current = true;
    setState((s) => (s?.phase === 'running' ? { ...s, hidden: true } : null));
  };
  const shown = state !== null && !(state.phase === 'running' && state.hidden);

  return {
    tasks,
    dialog: shown && (
      <FileTaskDialog
        state={state}
        workOffline={workOffline}
        onClose={close}
        {...(onOpenFolder
          ? {
              onOpenFolder: (folderId: string) => {
                setState(null);
                onOpenFolder(folderId);
              },
            }
          : {})}
      />
    ),
  };
}

function FileTaskDialog({
  state,
  workOffline,
  onClose,
  onOpenFolder,
}: {
  state: State;
  workOffline: boolean;
  onClose: () => void;
  onOpenFolder?: (folderId: string) => void;
}) {
  if (state.phase === 'running') return <ProgressDialog task={state.task} onClose={onClose} />;
  return <ResultDialog state={state} workOffline={workOffline} onClose={onClose} {...(onOpenFolder ? { onOpenFolder } : {})} />;
}

/**
 * While a task runs: its progress events, kept here so that only this
 * dialog redraws with each one. Nothing shows until the first event, so a
 * native picker is not covered, and a quick task goes straight to its result.
 */
function ProgressDialog({ task, onClose }: { task: FileTask; onClose: () => void }) {
  const api = useMailApi();
  const [progress, setProgress] = useState<ImportProgress | null>(null);
  useEffect(() => api.on('mail://import-progress', setProgress), [api]);
  usePhoneBack(progress ? 1 : 0, onClose);
  if (!progress) return null;
  const known = progress.total > 0;
  return (
    <Modal title={task.title} onClose={onClose} size="small">
      <div className="file-progress">
        <p className="file-progress__phase" role="status">
          {progress.phase}
          {known && <span className="muted"> · {`${String(progress.done)} of ${String(progress.total)}`}</span>}
        </p>
        {known ? (
          <progress
            className="file-progress__bar"
            aria-label={task.title}
            value={Math.min(progress.done, progress.total)}
            max={progress.total}
          />
        ) : (
          <progress className="file-progress__bar" aria-label={task.title} />
        )}
        <p className="form-hint">
          {task.long
            ? 'This can take a long time for a big file. You can close this; the import keeps going and a message here says when it is done.'
            : 'You can close this; it keeps going.'}
        </p>
      </div>
      <div className="modal__actions">
        <button type="button" className="btn btn--secondary" onClick={onClose}>
          Close
        </button>
      </div>
    </Modal>
  );
}

function ResultDialog({
  state,
  workOffline,
  onClose,
  onOpenFolder,
}: {
  state: Exclude<State, { phase: 'running' }>;
  workOffline: boolean;
  onClose: () => void;
  onOpenFolder?: (folderId: string) => void;
}) {
  usePhoneBack(1, onClose);
  const { task } = state;
  if (state.phase === 'failed') {
    const copy = fileErrorCopy(state.error, workOffline);
    return (
      <Modal title={task.title} onClose={onClose} size="small">
        <div className="notice notice--error" role="alert">
          <p className="notice__title">{copy.title}</p>
          <p>{copy.text}</p>
        </div>
        <div className="modal__actions">
          <button type="button" className="btn btn--primary" onClick={onClose}>
            OK
          </button>
        </div>
      </Modal>
    );
  }

  const { report } = state;
  const folderId = report.folderId ?? null;
  return (
    <Modal title={task.doneTitle ?? 'Import finished'} onClose={onClose} size="small">
      <div className="file-summary">
        <p className="file-summary__line" role="status">
          {reportLine(report, task.noun)}.
        </p>
        {report.counts && (
          <ul className="file-summary__counts" aria-label="Imported by kind">
            {KINDS.map(([key, word]) => (
              <li key={key}>
                <span>{`${word.charAt(0).toUpperCase()}${word.slice(1)}s`}</span>
                <span>{report.counts?.[key] ?? 0}</span>
              </li>
            ))}
          </ul>
        )}
        {report.skipped > 0 && <p>{countOf(report.skipped, 'item')} left out.</p>}
        {report.problems.length > 0 && (
          <details className="file-summary__problems">
            <summary>
              {report.problems.length === 1 ? 'Why one item was left out' : `Why ${String(report.problems.length)} items were left out`}
            </summary>
            <ul>
              {report.problems.map((problem, i) => (
                <li key={i}>{problem}</li>
              ))}
            </ul>
          </details>
        )}
      </div>
      <div className="modal__actions">
        {folderId !== null && onOpenFolder && (
          <button
            type="button"
            className="btn btn--secondary"
            onClick={() => {
              onOpenFolder(folderId);
            }}
          >
            Open folder
          </button>
        )}
        <button type="button" className="btn btn--primary" onClick={onClose}>
          Done
        </button>
      </div>
    </Modal>
  );
}
