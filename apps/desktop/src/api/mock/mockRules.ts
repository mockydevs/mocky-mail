import { appError } from '../errors';
import type { Account, Category, Folder, Rule, RuleAction, RuleCondition, RuleInput } from '../types';
import type { MockMessage } from './mockMailApi';

type StoredFolder = Omit<Folder, 'unread' | 'total'>;

const TEXT_KINDS = new Set<RuleCondition['kind']>(['from', 'to', 'cc', 'toOrCc', 'subject', 'body', 'subjectOrBody']);
const NO_VALUE = new Set<RuleCondition['kind']>(['hasAttachment', 'toMe', 'onlyToMe']);

export interface RuleWorld {
  accounts: readonly Account[];
  folders: readonly StoredFolder[];
  categories: readonly Category[];
}

/** The words of a text condition: comma-separated, any of them matches. */
function words(value: string): string[] {
  return value
    .split(',')
    .map((w) => w.trim().toLowerCase())
    .filter(Boolean);
}

function checkCondition(c: RuleCondition, world: RuleWorld, where: string): void {
  if (TEXT_KINDS.has(c.kind) && words(c.value).length === 0) {
    throw appError('invalid', `${where}: type the words or addresses to look for.`);
  }
  if (c.kind === 'header' && !/^[\w-]+\s*:\s*\S/.test(c.value)) {
    throw appError('invalid', `${where}: write the header and the words, like "List-Id: newsletter".`);
  }
  if (c.kind === 'importance' && !['high', 'normal', 'low'].includes(c.value)) {
    throw appError('invalid', `${where}: choose high, normal or low importance.`);
  }
  if ((c.kind === 'sizeOver' || c.kind === 'sizeUnder') && !(Number(c.value) > 0)) {
    throw appError('invalid', `${where}: give the size in KB, like 500.`);
  }
  if (c.kind === 'category' && !world.categories.some((k) => k.name.toLowerCase() === c.value.trim().toLowerCase())) {
    throw appError('invalid', `${where}: choose one of your categories.`);
  }
}

function checkAction(a: RuleAction, rule: RuleInput, world: RuleWorld): void {
  if (a.kind === 'move' || a.kind === 'copy') {
    const verb = a.kind === 'move' ? 'Move to' : 'Copy to';
    if (rule.accountId === null) {
      if (a.folderId) {
        throw appError('invalid', `${verb}: a rule for all accounts can only use a folder every account has, like Archive.`);
      }
      if (!a.role) throw appError('invalid', `${verb}: choose a folder.`);
      return;
    }
    const folder = a.folderId ? world.folders.find((f) => f.id === a.folderId) : undefined;
    if (!folder && !a.role) throw appError('invalid', `${verb}: choose a folder.`);
    if (folder && folder.accountId !== rule.accountId) throw appError('invalid', `${verb}: that folder is in another account.`);
  }
  if (a.kind === 'categorize' && !world.categories.some((k) => k.name.toLowerCase() === (a.category ?? '').toLowerCase())) {
    throw appError('invalid', 'Add category: choose one of your categories.');
  }
  if (a.kind === 'setImportance' && !a.importance) throw appError('invalid', 'Set importance: choose high, normal or low.');
}

/** What save_rule refuses, in the backend's words. */
export function validateRule(rule: RuleInput, world: RuleWorld): void {
  if (!rule.name.trim()) throw appError('invalid', 'Give the rule a name.');
  if (rule.accountId !== null && !world.accounts.some((a) => a.id === rule.accountId)) {
    throw appError('not_found', 'That account no longer exists.');
  }
  rule.conditions.forEach((c, i) => {
    checkCondition(c, world, `Condition ${String(i + 1)}`);
  });
  rule.exceptions.forEach((c, i) => {
    checkCondition(c, world, `Exception ${String(i + 1)}`);
  });
  if (rule.actions.length === 0) throw appError('invalid', 'Add at least one action: what should happen to the messages?');
  for (const a of rule.actions) checkAction(a, rule, world);
}

/** Whether the server's Sieve can run it too, and why not. */
export function serverStatusOf(rule: RuleInput): Pick<Rule, 'serverStatus' | 'serverNote'> {
  if (!rule.keepOnServer) return { serverStatus: 'local', serverNote: null };
  if (rule.accountId === null) {
    return { serverStatus: 'unsupported', serverNote: 'Rules for all accounts run on this device only. Pick one account to keep it on its server.' };
  }
  if ([...rule.conditions, ...rule.exceptions].some((c) => c.kind === 'category')) {
    return { serverStatus: 'unsupported', serverNote: "The mail server doesn't know your categories, so this rule runs on this device only." };
  }
  if (rule.actions.some((a) => a.kind === 'categorize' || a.kind === 'clearCategories' || a.kind === 'pin' || a.kind === 'notify')) {
    return {
      serverStatus: 'unsupported',
      serverNote: "The mail server can't categorise, pin or notify, so this rule runs on this device only.",
    };
  }
  return { serverStatus: 'server', serverNote: null };
}

function conditionMatches(m: MockMessage, c: RuleCondition, ownEmail: string): boolean {
  const any = (text: string) => words(c.value).some((w) => text.toLowerCase().includes(w));
  const people = (list: readonly { name: string | null; address: string }[]) =>
    list.map((p) => `${p.name ?? ''} <${p.address}>`).join(', ');
  const body = `${m.preview} ${m.text ?? ''} ${(m.html ?? '').replace(/<[^>]*>/g, ' ')}`;
  switch (c.kind) {
    case 'from':
      return any(m.from ? people([m.from]) : '');
    case 'to':
      return any(people(m.to));
    case 'cc':
      return any(people(m.cc));
    case 'toOrCc':
      return any(people([...m.to, ...m.cc]));
    case 'subject':
      return any(m.subject);
    case 'body':
      return any(body);
    case 'subjectOrBody':
      return any(`${m.subject} ${body}`);
    case 'header': {
      const at = c.value.indexOf(':');
      const name = c.value.slice(0, at).trim().toLowerCase();
      const want = c.value.slice(at + 1).trim().toLowerCase();
      const headers: Record<string, string> = {
        from: m.from ? people([m.from]) : '',
        to: people(m.to),
        cc: people(m.cc),
        subject: m.subject,
        'message-id': m.messageIdHeader ?? '',
      };
      return (headers[name] ?? '').toLowerCase().includes(want);
    }
    case 'hasAttachment':
      return m.hasAttachments;
    case 'importance':
      return m.importance === c.value;
    case 'sizeOver':
      return m.size > Number(c.value) * 1024;
    case 'sizeUnder':
      return m.size < Number(c.value) * 1024;
    case 'category':
      return m.categories.some((k) => k.toLowerCase() === c.value.trim().toLowerCase());
    case 'toMe':
      return [...m.to, ...m.cc].some((p) => p.address.toLowerCase() === ownEmail);
    case 'onlyToMe':
      return m.cc.length === 0 && m.to.length === 1 && m.to[0]?.address.toLowerCase() === ownEmail;
  }
}

/** Whether the rule picks this message: its conditions (all or any), and none of its exceptions. */
export function ruleMatches(rule: RuleInput, m: MockMessage, ownEmail: string): boolean {
  if (rule.accountId !== null && rule.accountId !== m.accountId) return false;
  const conditions = rule.conditions.filter((c) => NO_VALUE.has(c.kind) || c.value.trim() !== '');
  const hit = (c: RuleCondition) => conditionMatches(m, c, ownEmail.toLowerCase());
  const matched =
    conditions.length === 0 || (rule.match === 'all' ? conditions.every(hit) : conditions.some(hit));
  return matched && !rule.exceptions.filter((c) => NO_VALUE.has(c.kind) || c.value.trim() !== '').some(hit);
}
