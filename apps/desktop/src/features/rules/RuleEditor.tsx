import { useEffect, useId, useRef, useState, type SyntheticEvent } from 'react';

import { useMailApi } from '../../api/MailApiContext';
import { toAppError } from '../../api/errors';
import type {
  Account,
  AppError,
  Category,
  Folder,
  FolderRole,
  Importance,
  Rule,
  RuleAction,
  RuleCondition,
  RuleInput,
} from '../../api/types';
import { usePhoneBack } from '../../app/usePhone';
import { useDebounced } from '../../lib/useDebounced';
import { Icon } from '../../ui/Icon';
import { ConfirmDialog, Modal } from '../../ui/Modal';
import { FolderPicker } from '../mailbox/FolderPicker';
import { ROLE_TITLES } from '../mailbox/selectors';
import {
  ACTION_LABELS,
  blankAction,
  blankCondition,
  CONDITION_LABELS,
  IMPORTANCE_LABELS,
  RULE_ROLES,
  targetName,
  valueInputOf,
} from './ruleText';

/** How long the editor waits after a change before counting matches again. */
export const PREVIEW_DEBOUNCE_MS = 400;

interface Props {
  initial: RuleInput;
  accounts: readonly Account[];
  folders: Readonly<Record<string, Folder[]>>;
  categories: readonly Category[];
  onSaved: (rules: Rule[], saved: RuleInput) => void;
  onClose: () => void;
}

const isImportance = (v: string): v is Importance => v === 'high' || v === 'normal' || v === 'low';

/** Move and copy follow the rule to another account: by role where there is one, else the user picks again. */
function retarget(action: RuleAction, accountId: string | null, folders: Readonly<Record<string, Folder[]>>): RuleAction {
  if (action.kind !== 'move' && action.kind !== 'copy') return action;
  const all = Object.values(folders).flat();
  const role: FolderRole | null = action.role ?? all.find((f) => f.id === action.folderId)?.role ?? null;
  if (accountId === null) {
    return { kind: action.kind, folderId: null, role: role && RULE_ROLES.includes(role) ? role : null };
  }
  const same = all.find((f) => f.id === action.folderId && f.accountId === accountId);
  const byRole = role && role !== 'other' ? (folders[accountId] ?? []).find((f) => f.role === role) : undefined;
  return { kind: action.kind, folderId: (same ?? byRole)?.id ?? null, role: null };
}

/** Rows of conditions or exceptions: what to look at, and the value it needs. */
function ConditionList({
  noun,
  items,
  categories,
  onChange,
}: {
  noun: 'Condition' | 'Exception';
  items: readonly RuleCondition[];
  categories: readonly Category[];
  onChange: (items: RuleCondition[]) => void;
}) {
  const id = useId();
  const firstCategory = categories[0]?.name ?? '';
  const set = (index: number, next: RuleCondition) => {
    onChange(items.map((c, i) => (i === index ? next : c)));
  };
  return (
    <>
      {items.length > 0 && (
        <ol className="rule-rows" aria-label={noun === 'Condition' ? 'Conditions' : 'Exceptions'}>
          {items.map((c, index) => {
            const n = String(index + 1);
            const input = valueInputOf(c.kind);
            const valueId = `${id}-${n}-value`;
            return (
              <li key={index} className="rule-row">
                <select
                  className="field rule-row__kind"
                  aria-label={`${noun} ${n}`}
                  value={c.kind}
                  onChange={(e) => {
                    const kind = CONDITION_LABELS.find(([k]) => k === e.target.value)?.[0];
                    if (!kind) return;
                    // Keep typed words when switching between the text kinds.
                    const keep = valueInputOf(kind) === input && input !== 'none';
                    set(index, keep ? { kind, value: c.value } : blankCondition(kind, firstCategory));
                  }}
                >
                  {CONDITION_LABELS.map(([kind, label]) => (
                    <option key={kind} value={kind}>
                      {label}
                    </option>
                  ))}
                </select>
                {(input === 'text' || input === 'header') && (
                  <input
                    id={valueId}
                    className="field rule-row__value"
                    aria-label={`${noun} ${n}: ${input === 'header' ? 'header and words' : 'words or addresses'}`}
                    placeholder={input === 'header' ? 'List-Id: newsletter' : 'Separate several with commas'}
                    value={c.value}
                    onChange={(e) => {
                      set(index, { kind: c.kind, value: e.target.value });
                    }}
                  />
                )}
                {input === 'importance' && (
                  <select
                    className="field rule-row__value"
                    aria-label={`${noun} ${n}: importance`}
                    value={c.value}
                    onChange={(e) => {
                      set(index, { kind: c.kind, value: e.target.value });
                    }}
                  >
                    {IMPORTANCE_LABELS.map(([value, label]) => (
                      <option key={value} value={value}>
                        {label}
                      </option>
                    ))}
                  </select>
                )}
                {input === 'size' && (
                  <span className="rule-row__value rule-row__size">
                    <input
                      className="field"
                      type="number"
                      min={1}
                      inputMode="numeric"
                      aria-label={`${noun} ${n}: size in KB`}
                      value={c.value}
                      onChange={(e) => {
                        set(index, { kind: c.kind, value: e.target.value });
                      }}
                    />
                    <span aria-hidden="true">KB</span>
                  </span>
                )}
                {input === 'category' && (
                  <select
                    className="field rule-row__value"
                    aria-label={`${noun} ${n}: category`}
                    value={c.value}
                    disabled={categories.length === 0}
                    onChange={(e) => {
                      set(index, { kind: c.kind, value: e.target.value });
                    }}
                  >
                    {categories.length === 0 && <option value="">No categories yet</option>}
                    {categories.map((k) => (
                      <option key={k.name} value={k.name}>
                        {k.name}
                      </option>
                    ))}
                  </select>
                )}
                <button
                  type="button"
                  className="icon-btn rule-row__remove"
                  aria-label={`Remove ${noun.toLowerCase()} ${n}`}
                  title="Remove"
                  onClick={() => {
                    onChange(items.filter((_, i) => i !== index));
                  }}
                >
                  <Icon name="close" size={16} />
                </button>
              </li>
            );
          })}
        </ol>
      )}
      <button
        type="button"
        className="btn btn--secondary btn--small"
        onClick={() => {
          onChange([...items, blankCondition('from', firstCategory)]);
        }}
      >
        <Icon name="plus" size={16} />
        {noun === 'Condition' ? 'Add condition' : 'Add exception'}
      </button>
    </>
  );
}

/**
 * Make or change a mail rule: which account, what it looks for, what it
 * skips, what it does. Counts the Inbox messages it would match as it is
 * edited. On a phone it fills the screen; Back and Esc ask before throwing
 * changes away.
 */
export function RuleEditor({ initial, accounts, folders, categories, onSaved, onClose }: Props) {
  const api = useMailApi();
  const id = useId();
  const [rule, setRule] = useState<RuleInput>(initial);
  const [error, setError] = useState<AppError | null>(null);
  const [busy, setBusy] = useState(false);
  const [confirmLeave, setConfirmLeave] = useState(false);
  /** The action whose folder is being picked. */
  const [picking, setPicking] = useState<number | null>(null);
  const dirty = JSON.stringify(rule) !== JSON.stringify(initial);
  const firstCategory = categories[0]?.name ?? null;

  const requestClose = () => {
    if (dirty) setConfirmLeave(true);
    else onClose();
  };
  usePhoneBack(1, () => {
    if (confirmLeave) setConfirmLeave(false);
    else if (picking !== null) setPicking(null);
    else requestClose();
  });

  const change = (patch: Partial<RuleInput>) => {
    setRule((r) => ({ ...r, ...patch }));
    setError(null);
  };
  const setAction = (index: number, next: RuleAction) => {
    change({ actions: rule.actions.map((a, i) => (i === index ? next : a)) });
  };

  // Live count of what the rule would catch in the Inbox (the first account's, for all accounts).
  const previewAccount = rule.accountId ?? accounts[0]?.id ?? null;
  const previewFolder = previewAccount === null ? undefined : (folders[previewAccount] ?? []).find((f) => f.role === 'inbox');
  const settled = useDebounced(rule, PREVIEW_DEBOUNCE_MS);
  const settledKey = `${JSON.stringify(settled)}|${previewFolder?.id ?? ''}`;
  const [preview, setPreview] = useState<{ key: string; count: number | null; error: string | null } | null>(null);
  const ticket = useRef(0);
  useEffect(() => {
    const mine = ++ticket.current;
    if (!previewFolder) return;
    api.previewRule(settled, previewFolder.id).then(
      (count) => {
        if (mine === ticket.current) setPreview({ key: settledKey, count, error: null });
      },
      (e: unknown) => {
        if (mine === ticket.current) setPreview({ key: settledKey, count: null, error: toAppError(e).message });
      },
    );
    // settledKey stands for `settled` and the folder.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [api, settledKey]);
  const previewAccountName = accounts.find((a) => a.id === previewAccount);
  const where = rule.accountId === null && previewAccountName
    ? `in ${previewAccountName.displayName || previewAccountName.email}’s Inbox (this rule covers all accounts; the first one is counted)`
    : 'in Inbox';
  const previewText = !previewFolder
    ? null
    : preview?.key !== settledKey
      ? 'Counting matching messages…'
      : preview.error !== null
        ? `Can’t count matches yet: ${preview.error}`
        : `Matches ${String(preview.count)} ${preview.count === 1 ? 'message' : 'messages'} ${where}`;

  const save = (event: SyntheticEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (busy) return;
    setBusy(true);
    setError(null);
    api.saveRule(rule).then(
      (rules) => {
        setBusy(false);
        onSaved(rules, rule);
      },
      (e: unknown) => {
        setBusy(false);
        setError(toAppError(e));
      },
    );
  };

  const accountFolders = rule.accountId === null ? [] : (folders[rule.accountId] ?? []);
  const picked = picking === null ? undefined : rule.actions[picking];
  const pickingAccount = accounts.find((a) => a.id === rule.accountId);

  return (
    <>
      <Modal
        title={initial.id === null ? 'New rule' : `Edit rule`}
        description="Rules sort new mail as it arrives. You can also run them on a folder from Settings."
        size="large"
        onClose={requestClose}
      >
        <form className="rule-editor" aria-label="Rule" onSubmit={save} noValidate>
          <label className="form-field" htmlFor={`${id}-name`}>
            <span className="form-field__label">Name</span>
            <input
              id={`${id}-name`}
              className="field"
              value={rule.name}
              placeholder="For example: Invoices to Accounts"
              onChange={(e) => {
                change({ name: e.target.value });
              }}
            />
          </label>

          <label className="form-field" htmlFor={`${id}-account`}>
            <span className="form-field__label">Account</span>
            <select
              id={`${id}-account`}
              className="field"
              value={rule.accountId ?? ''}
              onChange={(e) => {
                const accountId = e.target.value === '' ? null : e.target.value;
                change({ accountId, actions: rule.actions.map((a) => retarget(a, accountId, folders)) });
              }}
            >
              {accounts.map((a) => (
                <option key={a.id} value={a.id}>
                  {a.displayName || a.email}
                </option>
              ))}
              <option value="">All accounts</option>
            </select>
          </label>

          <fieldset className="rule-editor__group">
            <legend className="form-field__label">When a message matches</legend>
            <div className="choice-row">
              <label className="choice">
                <input
                  type="radio"
                  name={`${id}-match`}
                  checked={rule.match === 'all'}
                  onChange={() => {
                    change({ match: 'all' });
                  }}
                />
                All of these conditions
              </label>
              <label className="choice">
                <input
                  type="radio"
                  name={`${id}-match`}
                  checked={rule.match === 'any'}
                  onChange={() => {
                    change({ match: 'any' });
                  }}
                />
                Any of them
              </label>
            </div>
            <ConditionList
              noun="Condition"
              items={rule.conditions}
              categories={categories}
              onChange={(conditions) => {
                change({ conditions });
              }}
            />
            {rule.conditions.length === 0 && <p className="form-hint">No conditions: the rule applies to every message.</p>}
          </fieldset>

          <fieldset className="rule-editor__group">
            <legend className="form-field__label">Except when</legend>
            {rule.exceptions.length === 0 && <p className="form-hint">Messages that match an exception are left alone.</p>}
            <ConditionList
              noun="Exception"
              items={rule.exceptions}
              categories={categories}
              onChange={(exceptions) => {
                change({ exceptions });
              }}
            />
          </fieldset>

          <fieldset className="rule-editor__group">
            <legend className="form-field__label">Do this</legend>
            {rule.actions.length > 0 && (
              <ol className="rule-rows" aria-label="Actions">
                {rule.actions.map((a, index) => {
                  const n = String(index + 1);
                  return (
                    <li key={index} className="rule-row">
                      <select
                        className="field rule-row__kind"
                        aria-label={`Action ${n}`}
                        value={a.kind}
                        onChange={(e) => {
                          const kind = ACTION_LABELS.find(([k]) => k === e.target.value)?.[0];
                          if (!kind) return;
                          const keepFolder = (kind === 'move' || kind === 'copy') && (a.kind === 'move' || a.kind === 'copy');
                          setAction(index, keepFolder ? { ...a, kind } : blankAction(kind, firstCategory));
                        }}
                      >
                        {ACTION_LABELS.map(([kind, label]) => (
                          <option key={kind} value={kind}>
                            {label}
                          </option>
                        ))}
                      </select>
                      {(a.kind === 'move' || a.kind === 'copy') &&
                        (rule.accountId === null ? (
                          <select
                            className="field rule-row__value"
                            aria-label={`Action ${n}: folder`}
                            value={a.role ?? ''}
                            onChange={(e) => {
                              const role = RULE_ROLES.find((r) => r === e.target.value) ?? null;
                              setAction(index, { kind: a.kind, folderId: null, role });
                            }}
                          >
                            <option value="">Choose a folder</option>
                            {RULE_ROLES.map((role) => (
                              <option key={role} value={role}>
                                {ROLE_TITLES[role]}
                              </option>
                            ))}
                          </select>
                        ) : (
                          <button
                            type="button"
                            className="btn btn--secondary rule-row__value"
                            aria-label={`Action ${n}: folder, ${a.folderId || a.role ? targetName(a, accountFolders) : 'none chosen'}`}
                            onClick={() => {
                              setPicking(index);
                            }}
                          >
                            <Icon name="folder" size={16} />
                            {a.folderId || a.role ? targetName(a, accountFolders) : 'Choose a folder…'}
                          </button>
                        ))}
                      {a.kind === 'categorize' && (
                        <select
                          className="field rule-row__value"
                          aria-label={`Action ${n}: category`}
                          value={a.category ?? ''}
                          disabled={categories.length === 0}
                          onChange={(e) => {
                            setAction(index, { kind: 'categorize', category: e.target.value || null });
                          }}
                        >
                          {categories.length === 0 && <option value="">No categories yet</option>}
                          {categories.map((k) => (
                            <option key={k.name} value={k.name}>
                              {k.name}
                            </option>
                          ))}
                        </select>
                      )}
                      {a.kind === 'setImportance' && (
                        <select
                          className="field rule-row__value"
                          aria-label={`Action ${n}: importance`}
                          value={a.importance ?? 'normal'}
                          onChange={(e) => {
                            const value = e.target.value;
                            if (isImportance(value)) setAction(index, { kind: 'setImportance', importance: value });
                          }}
                        >
                          {IMPORTANCE_LABELS.map(([value, label]) => (
                            <option key={value} value={value}>
                              {label}
                            </option>
                          ))}
                        </select>
                      )}
                      <button
                        type="button"
                        className="icon-btn rule-row__remove"
                        aria-label={`Remove action ${n}`}
                        title="Remove"
                        onClick={() => {
                          change({ actions: rule.actions.filter((_, i) => i !== index) });
                        }}
                      >
                        <Icon name="close" size={16} />
                      </button>
                    </li>
                  );
                })}
              </ol>
            )}
            <button
              type="button"
              className="btn btn--secondary btn--small"
              onClick={() => {
                change({ actions: [...rule.actions, blankAction(rule.actions.length === 0 ? 'move' : 'flag', firstCategory)] });
              }}
            >
              <Icon name="plus" size={16} />
              Add action
            </button>
            {rule.accountId === null && rule.actions.some((a) => a.kind === 'move' || a.kind === 'copy') && (
              <p className="form-hint">A rule for all accounts can only move or copy to a folder every account has.</p>
            )}
          </fieldset>

          <label className="choice">
            <input
              type="checkbox"
              checked={rule.stopProcessing}
              onChange={(e) => {
                change({ stopProcessing: e.target.checked });
              }}
            />
            Stop processing more rules
          </label>
          <p className="form-hint">Rules lower in the list don’t run on a message this rule matched.</p>

          <label className="choice">
            <input
              type="checkbox"
              checked={rule.keepOnServer}
              aria-describedby={`${id}-server`}
              onChange={(e) => {
                change({ keepOnServer: e.target.checked });
              }}
            />
            Keep on the mail server
          </label>
          <p id={`${id}-server`} className="form-hint">
            Also saves the rule on your mail server, so it sorts mail even while this device is off. Servers can’t run
            every condition or action; such a rule runs on this device only.
          </p>

          <label className="choice">
            <input
              type="checkbox"
              checked={rule.enabled}
              onChange={(e) => {
                change({ enabled: e.target.checked });
              }}
            />
            Turn this rule on
          </label>

          {previewText && (
            <p className="rule-editor__preview" role="status">
              {previewText}
            </p>
          )}
          {error && (
            <p className="form-error" role="alert">
              {error.message}
            </p>
          )}

          <div className="modal__actions">
            <button type="button" className="btn btn--secondary" onClick={requestClose}>
              Cancel
            </button>
            <button type="submit" className="btn btn--primary" disabled={busy}>
              {busy ? 'Saving' : 'Save rule'}
            </button>
          </div>
        </form>
      </Modal>
      {picking !== null && picked && rule.accountId !== null && (
        <FolderPicker
          mode="pick"
          accountIds={[rule.accountId]}
          title={picked.kind === 'copy' ? 'Copy to which folder?' : 'Move to which folder?'}
          description={pickingAccount ? `Folders in ${pickingAccount.displayName || pickingAccount.email}.` : ''}
          accounts={accounts}
          folders={folders}
          onClose={() => {
            setPicking(null);
          }}
          onPick={(target) => {
            const index = picking;
            setPicking(null);
            if ('folderId' in target) setAction(index, { kind: picked.kind, folderId: target.folderId, role: null });
          }}
        />
      )}
      {confirmLeave && (
        <ConfirmDialog
          title="Discard changes to this rule?"
          description="What you changed here is not saved."
          confirmLabel="Discard"
          danger
          onCancel={() => {
            setConfirmLeave(false);
          }}
          onConfirm={() => {
            setConfirmLeave(false);
            onClose();
          }}
        />
      )}
    </>
  );
}
