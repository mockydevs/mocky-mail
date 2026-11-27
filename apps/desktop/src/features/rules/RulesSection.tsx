import { useEffect, useId, useState } from 'react';

import { useMailApi } from '../../api/MailApiContext';
import { toAppError } from '../../api/errors';
import type { Account, AppError, Category, Folder, Rule, RuleInput } from '../../api/types';
import type { Notice } from '../../app/useToasts';
import { useMailEvent } from '../../lib/useEvent';
import { Icon } from '../../ui/Icon';
import { ConfirmDialog } from '../../ui/Modal';
import { FolderPicker } from '../mailbox/FolderPicker';
import { RuleEditor } from './RuleEditor';
import { accountName, blankRule, inputOf, ruleSummary } from './ruleText';

interface Props {
  accounts: readonly Account[];
  folders: Readonly<Record<string, Folder[]>>;
  categories: readonly Category[];
  /** Phones: no dragging; the Move up and Move down buttons do it. */
  phone?: boolean;
  /** A toast, for "Checked N, matched M". */
  onNotice?: (notice: Notice) => void;
}

const BADGES: Readonly<Record<Rule['serverStatus'], { text: string; tone: string; title: string }>> = {
  server: { text: 'On server', tone: 'server', title: 'Saved on the mail server too: it sorts mail even while this device is off.' },
  local: { text: 'This device only', tone: 'local', title: 'Runs in Mocky Mail on this device.' },
  unsupported: { text: 'This device only', tone: 'unsupported', title: '' },
};

/** Kept on the server, but the server copy is still being written (the real status follows in mail://rules-changed). */
function savingToServer(rule: Rule): boolean {
  return rule.serverStatus === 'local' && rule.keepOnServer && rule.serverNote !== null;
}

/** Where a "Run now" goes: one rule, or every rule (null). */
interface Running {
  rule: Rule | null;
}

/** `ids` with `moved` put where `target` is. */
function reorder(ids: readonly string[], moved: string, target: string): string[] {
  const without = ids.filter((id) => id !== moved);
  const at = without.indexOf(target);
  const after = ids.indexOf(moved) < ids.indexOf(target);
  without.splice(after ? at + 1 : at, 0, moved);
  return without;
}

/**
 * Settings › Rules: the rules in the order they run, each with a switch, a
 * summary and where it runs (this device or the server too). Reorder by
 * dragging or with Move up and Move down; run any of them on a folder now.
 */
export function RulesSection({ accounts, folders, categories, phone = false, onNotice }: Props) {
  const api = useMailApi();
  const id = useId();
  const [rules, setRules] = useState<Rule[] | null>(null);
  const [error, setError] = useState<AppError | null>(null);
  const [done, setDone] = useState<string | null>(null);
  const [editing, setEditing] = useState<RuleInput | null>(null);
  const [deleting, setDeleting] = useState<Rule | null>(null);
  const [running, setRunning] = useState<Running | null>(null);
  const [dragging, setDragging] = useState<string | null>(null);
  const [over, setOver] = useState<string | null>(null);
  const allFolders = Object.values(folders).flat();

  useEffect(() => {
    api.listRules().then(setRules, (e: unknown) => {
      setError(toAppError(e));
      setRules([]);
    });
  }, [api]);
  // A server copy was written, or writing it gave up: the statuses change.
  useMailEvent(api, 'mail://rules-changed', (payload) => {
    setRules(payload.rules);
  });

  const fail = (e: unknown) => {
    setError(toAppError(e));
  };
  const apply = (work: Promise<Rule[]>, success: string | null, rollback?: Rule[]) => {
    setError(null);
    setDone(null);
    work.then(
      (next) => {
        setRules(next);
        if (success) setDone(success);
      },
      (e: unknown) => {
        if (rollback) setRules(rollback);
        fail(e);
      },
    );
  };

  const move = (ids: string[]) => {
    if (!rules) return;
    const before = rules;
    // Show the new order at once; the backend's answer follows.
    setRules(ids.map((x) => rules.find((r) => r.id === x)).filter((r): r is Rule => r !== undefined));
    apply(api.reorderRules(ids), null, before);
  };
  const step = (rule: Rule, by: -1 | 1) => {
    if (!rules) return;
    const ids = rules.map((r) => r.id);
    const from = ids.indexOf(rule.id);
    const to = from + by;
    if (to < 0 || to >= ids.length) return;
    ids.splice(from, 1);
    ids.splice(to, 0, rule.id);
    move(ids);
  };

  const run = (rule: Rule | null, folderId: string, folderName: string) => {
    setError(null);
    api.runRules(rule ? [rule.id] : null, folderId).then(
      (result) => {
        const text = `Checked ${String(result.checked)}, matched ${String(result.matched)}.`;
        const title = rule ? `Ran “${rule.name}” on ${folderName}` : `Ran your rules on ${folderName}`;
        if (onNotice) onNotice({ tone: 'info', title, text });
        else setDone(`${title}: ${text}`);
      },
      fail,
    );
  };

  const list = rules ?? [];
  const runAccounts = running?.rule?.accountId ? [running.rule.accountId] : accounts.map((a) => a.id);

  return (
    <section className="settings__group settings__group--wide" aria-labelledby={`${id}-title`}>
      <h2 id={`${id}-title`} className="settings__legend">
        Rules
      </h2>
      <p className="form-hint">
        Rules sort new mail as it arrives, from the top of the list down. {phone ? '' : 'Drag a rule to change the order.'}
      </p>
      {rules === null ? (
        <p className="muted" aria-busy="true">
          Loading rules
        </p>
      ) : list.length === 0 ? (
        <p className="muted">No rules yet. Make one here, or with “Create rule…” on a message.</p>
      ) : (
        <ol className="rule-list" aria-labelledby={`${id}-title`}>
          {list.map((rule, index) => {
            const saving = savingToServer(rule);
            const badge = saving
              ? { text: 'Saving to server…', tone: 'saving', title: rule.serverNote ?? '' }
              : BADGES[rule.serverStatus];
            const note =
              rule.serverStatus === 'unsupported' ? (rule.serverNote ?? 'The server can’t run this rule.') : rule.serverNote;
            const dropHere = dragging !== null && dragging !== rule.id;
            return (
              // The row is where a dragged rule is dropped; the keyboard and
              // phones use the Move up and Move down buttons instead.
              // eslint-disable-next-line jsx-a11y/no-noninteractive-element-interactions
              <li
                key={rule.id}
                className={`rule-item${rule.enabled ? '' : ' rule-item--off'}${dropHere && over === rule.id ? ' rule-item--drop' : ''}${dragging === rule.id ? ' rule-item--dragging' : ''}`}
                onDragOver={(e) => {
                  if (!dropHere) return;
                  e.preventDefault();
                  setOver(rule.id);
                }}
                onDragLeave={() => {
                  setOver((current) => (current === rule.id ? null : current));
                }}
                onDrop={(e) => {
                  if (!dropHere) return;
                  e.preventDefault();
                  const moved = dragging;
                  setDragging(null);
                  setOver(null);
                  move(reorder(list.map((r) => r.id), moved, rule.id));
                }}
              >
                <div className="rule-item__main">
                  {!phone && (
                    <button
                      type="button"
                      className="icon-btn rule-item__handle"
                      aria-label={`Drag ${rule.name} to change the order`}
                      title="Drag to change the order"
                      draggable
                      onDragStart={(e) => {
                        setDragging(rule.id);
                        const transfer = e.dataTransfer as DataTransfer | null | undefined;
                        if (transfer) {
                          transfer.effectAllowed = 'move';
                          transfer.setData('text/plain', rule.name);
                        }
                      }}
                      onDragEnd={() => {
                        setDragging(null);
                        setOver(null);
                      }}
                    >
                      <Icon name="menu" size={16} />
                    </button>
                  )}
                  <input
                    type="checkbox"
                    className="rule-item__switch"
                    checked={rule.enabled}
                    aria-label={`Use ${rule.name}`}
                    title={rule.enabled ? 'On: turn this rule off' : 'Off: turn this rule on'}
                    onChange={(e) => {
                      apply(api.saveRule({ ...inputOf(rule), enabled: e.target.checked }), null);
                    }}
                  />
                  <div className="rule-item__text">
                    <p className="rule-item__name">
                      {rule.name}
                      {!rule.enabled && <span className="muted small"> (off)</span>}
                    </p>
                    <p className="rule-item__summary">{ruleSummary(rule, allFolders)}</p>
                    <p className="rule-item__meta muted small">
                      <span>{accountName(rule.accountId, accounts)}</span>
                      <span
                        className={`badge rule-badge rule-badge--${badge.tone}`}
                        title={note ?? badge.title}
                      >
                        {badge.text}
                      </span>
                    </p>
                    {note && <p className="form-hint rule-item__note">{note}</p>}
                  </div>
                </div>
                <div className="rule-item__actions">
                  <button
                    type="button"
                    className="icon-btn"
                    aria-label={`Move ${rule.name} up`}
                    title="Move up"
                    disabled={index === 0}
                    onClick={() => {
                      step(rule, -1);
                    }}
                  >
                    <span className="flip" aria-hidden="true">
                      <Icon name="chevron" />
                    </span>
                  </button>
                  <button
                    type="button"
                    className="icon-btn"
                    aria-label={`Move ${rule.name} down`}
                    title="Move down"
                    disabled={index === list.length - 1}
                    onClick={() => {
                      step(rule, 1);
                    }}
                  >
                    <Icon name="chevron" />
                  </button>
                  <button
                    type="button"
                    className="btn btn--link"
                    aria-label={`Edit ${rule.name}`}
                    onClick={() => {
                      setDone(null);
                      setEditing(inputOf(rule));
                    }}
                  >
                    Edit
                  </button>
                  <button
                    type="button"
                    className="btn btn--link"
                    aria-label={`Run ${rule.name} now…`}
                    onClick={() => {
                      setDone(null);
                      setRunning({ rule });
                    }}
                  >
                    Run now…
                  </button>
                  <button
                    type="button"
                    className="btn btn--link btn--danger-text"
                    aria-label={`Delete ${rule.name}`}
                    onClick={() => {
                      setDone(null);
                      setDeleting(rule);
                    }}
                  >
                    Delete
                  </button>
                </div>
              </li>
            );
          })}
        </ol>
      )}
      <div className="choice-row">
        <button
          type="button"
          className="btn btn--secondary"
          disabled={accounts.length === 0}
          onClick={() => {
            setDone(null);
            setEditing(blankRule(accounts[0]?.id ?? null));
          }}
        >
          <Icon name="plus" />
          New rule
        </button>
        {list.some((r) => r.enabled) && (
          <button
            type="button"
            className="btn btn--secondary"
            onClick={() => {
              setDone(null);
              setRunning({ rule: null });
            }}
          >
            Run all rules now…
          </button>
        )}
      </div>
      {error && (
        <p className="form-error" role="alert">
          {error.message}
        </p>
      )}
      {done && (
        <p className="form-hint" role="status">
          {done}
        </p>
      )}

      {editing && (
        <RuleEditor
          initial={editing}
          accounts={accounts}
          folders={folders}
          categories={categories}
          onClose={() => {
            setEditing(null);
          }}
          onSaved={(next, saved) => {
            setEditing(null);
            setRules(next);
            setError(null);
            setDone(saved.id === null ? `Rule “${saved.name.trim()}” added.` : `Rule “${saved.name.trim()}” saved.`);
          }}
        />
      )}
      {deleting && (
        <ConfirmDialog
          title={`Delete ${deleting.name}?`}
          description={
            deleting.serverStatus === 'server'
              ? 'It stops sorting mail here and is taken off the mail server. Messages it already sorted stay where they are.'
              : 'It stops sorting mail. Messages it already sorted stay where they are.'
          }
          confirmLabel="Delete rule"
          danger
          onCancel={() => {
            setDeleting(null);
          }}
          onConfirm={() => {
            const target = deleting;
            setDeleting(null);
            apply(api.deleteRule(target.id), `Rule “${target.name}” deleted.`);
          }}
        />
      )}
      {running && (
        <FolderPicker
          mode="pick"
          accountIds={runAccounts}
          title={running.rule ? `Run “${running.rule.name}” on which folder?` : 'Run your rules on which folder?'}
          description={
            running.rule
              ? 'The rule looks at every message in the folder now, even read ones.'
              : 'Every rule that is on looks at every message in the folder now, top of the list first.'
          }
          accounts={accounts}
          folders={folders}
          onClose={() => {
            setRunning(null);
          }}
          onPick={(target, name) => {
            const rule = running.rule;
            setRunning(null);
            if ('folderId' in target) run(rule, target.folderId, name);
          }}
        />
      )}
    </section>
  );
}
