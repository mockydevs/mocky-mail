import type {
  Account,
  Folder,
  FolderRole,
  Importance,
  MessageListItem,
  Rule,
  RuleAction,
  RuleActionKind,
  RuleCondition,
  RuleConditionKind,
  RuleInput,
} from '../../api/types';
import { folderLabel, ROLE_TITLES } from '../mailbox/selectors';

/** What a condition looks at, in the editor's words, in the order offered. */
export const CONDITION_LABELS: readonly [RuleConditionKind, string][] = [
  ['from', 'From contains'],
  ['to', 'To contains'],
  ['cc', 'Cc contains'],
  ['toOrCc', 'To or Cc contains'],
  ['subject', 'Subject contains'],
  ['body', 'Message body contains'],
  ['subjectOrBody', 'Subject or body contains'],
  ['header', 'A header contains'],
  ['hasAttachment', 'Has an attachment'],
  ['importance', 'Importance is'],
  ['sizeOver', 'Larger than'],
  ['sizeUnder', 'Smaller than'],
  ['category', 'Has the category'],
  ['toMe', 'Sent to me'],
  ['onlyToMe', 'Sent only to me'],
];

export const ACTION_LABELS: readonly [RuleActionKind, string][] = [
  ['move', 'Move to'],
  ['copy', 'Copy to'],
  ['categorize', 'Add the category'],
  ['flag', 'Flag it'],
  ['clearFlag', 'Remove the flag'],
  ['clearCategories', 'Remove all categories'],
  ['markRead', 'Mark as read'],
  ['setImportance', 'Set importance to'],
  ['pin', 'Pin it'],
  ['notify', 'Notify me'],
  ['delete', 'Delete it (move to Trash)'],
];

export const IMPORTANCE_LABELS: readonly [Importance, string][] = [
  ['high', 'High'],
  ['normal', 'Normal'],
  ['low', 'Low'],
];

/** Folders every account has: what a rule for all accounts can move or copy to. */
export const RULE_ROLES: readonly FolderRole[] = ['inbox', 'archive', 'junk', 'trash'];

export type ValueInput = 'text' | 'header' | 'importance' | 'size' | 'category' | 'none';

/** The kind of value field a condition needs. */
export function valueInputOf(kind: RuleConditionKind): ValueInput {
  switch (kind) {
    case 'header':
      return 'header';
    case 'importance':
      return 'importance';
    case 'sizeOver':
    case 'sizeUnder':
      return 'size';
    case 'category':
      return 'category';
    case 'hasAttachment':
    case 'toMe':
    case 'onlyToMe':
      return 'none';
    case 'from':
    case 'to':
    case 'cc':
    case 'toOrCc':
    case 'subject':
    case 'body':
    case 'subjectOrBody':
      return 'text';
  }
}

/** A fresh condition of `kind`, with a sensible value for its field. */
export function blankCondition(kind: RuleConditionKind, firstCategory = ''): RuleCondition {
  const input = valueInputOf(kind);
  return { kind, value: input === 'importance' ? 'high' : input === 'category' ? firstCategory : '' };
}

export function blankAction(kind: RuleActionKind, firstCategory: string | null = null): RuleAction {
  switch (kind) {
    case 'move':
    case 'copy':
      return { kind, folderId: null, role: null };
    case 'categorize':
      return { kind, category: firstCategory };
    case 'setImportance':
      return { kind, importance: 'high' };
    default:
      return { kind };
  }
}

export function blankRule(accountId: string | null): RuleInput {
  return {
    id: null,
    name: '',
    enabled: true,
    accountId,
    match: 'all',
    conditions: [{ kind: 'from', value: '' }],
    exceptions: [],
    actions: [{ kind: 'move', folderId: null, role: null }],
    stopProcessing: false,
    keepOnServer: false,
  };
}

/** A saved rule as the editor and save_rule take it. */
export function inputOf(rule: Rule): RuleInput {
  return {
    id: rule.id,
    name: rule.name,
    enabled: rule.enabled,
    accountId: rule.accountId,
    match: rule.match,
    conditions: rule.conditions,
    exceptions: rule.exceptions,
    actions: rule.actions,
    stopProcessing: rule.stopProcessing,
    keepOnServer: rule.keepOnServer,
  };
}

/**
 * "Create rule…" from messages: their senders as a From condition, and a
 * move to the folder they are in now, unless that is the Inbox (then the
 * user picks one).
 */
export function ruleFromMessages(rows: readonly MessageListItem[], folders: readonly Folder[]): RuleInput {
  const accountIds = [...new Set(rows.map((r) => r.accountId))];
  const accountId = accountIds.length === 1 ? (accountIds[0] ?? null) : null;
  const senders = [...new Set(rows.map((r) => r.from?.address.toLowerCase()).filter((a): a is string => !!a))];
  const first = rows[0]?.from;
  const name = senders.length === 1 && first ? `Mail from ${first.name ?? first.address}` : 'Mail from these senders';
  const folderIds = [...new Set(rows.map((r) => r.folderId))];
  const folder = folderIds.length === 1 ? folders.find((f) => f.id === folderIds[0]) : undefined;
  const move: RuleAction =
    accountId === null
      ? { kind: 'move', folderId: null, role: folder && RULE_ROLES.includes(folder.role) && folder.role !== 'inbox' ? folder.role : null }
      : { kind: 'move', folderId: folder && folder.role !== 'inbox' ? folder.id : null, role: null };
  return {
    ...blankRule(accountId),
    name,
    conditions: [{ kind: 'from', value: senders.join(', ') }],
    actions: [move],
  };
}

const shorten = (text: string, max = 24) => (text.length > max ? `${text.slice(0, max - 1)}…` : text);

function conditionText(c: RuleCondition): string {
  const label = CONDITION_LABELS.find(([k]) => k === c.kind)?.[1] ?? c.kind;
  switch (valueInputOf(c.kind)) {
    case 'none':
      return label;
    case 'size':
      return `${label} ${c.value || '?'} KB`;
    case 'importance':
      return `${label} ${c.value}`;
    default:
      return `${label} ${shorten(c.value.trim()) || '…'}`;
  }
}

/** Where a move or copy goes, by name. */
export function targetName(action: RuleAction, folders: readonly Folder[]): string {
  if (action.role) return ROLE_TITLES[action.role];
  const folder = action.folderId ? folders.find((f) => f.id === action.folderId) : undefined;
  return folder ? folderLabel(folder) : 'a folder';
}

function actionText(a: RuleAction, folders: readonly Folder[]): string {
  switch (a.kind) {
    case 'move':
      return `move to ${targetName(a, folders)}`;
    case 'copy':
      return `copy to ${targetName(a, folders)}`;
    case 'categorize':
      return `add category ${a.category ?? '?'}`;
    case 'flag':
      return 'flag';
    case 'clearFlag':
      return 'remove flag';
    case 'clearCategories':
      return 'remove categories';
    case 'markRead':
      return 'mark as read';
    case 'setImportance':
      return `set importance ${a.importance ?? 'normal'}`;
    case 'pin':
      return 'pin';
    case 'notify':
      return 'notify me';
    case 'delete':
      return 'delete';
  }
}

/** One line: "From contains boss@… → move to Work, flag". */
export function ruleSummary(rule: RuleInput, folders: readonly Folder[]): string {
  const join = rule.match === 'all' ? ' and ' : ' or ';
  const when = rule.conditions.length === 0 ? 'Every message' : rule.conditions.map(conditionText).join(join);
  const except = rule.exceptions.length > 0 ? `, except ${rule.exceptions.map((c) => conditionText(c).toLowerCase()).join(' or ')}` : '';
  const what = rule.actions.length === 0 ? 'nothing yet' : rule.actions.map((a) => actionText(a, folders)).join(', ');
  return `${when}${except} → ${what}${rule.stopProcessing ? ', then stop' : ''}`;
}

export function accountName(accountId: string | null, accounts: readonly Account[]): string {
  if (accountId === null) return 'All accounts';
  const account = accounts.find((a) => a.id === accountId);
  return account ? account.displayName || account.email : 'Removed account';
}
