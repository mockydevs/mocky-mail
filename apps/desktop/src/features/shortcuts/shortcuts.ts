export type ShortcutAction =
  | 'next'
  | 'previous'
  | 'open'
  | 'reply'
  | 'replyAll'
  | 'forward'
  | 'archive'
  | 'delete'
  | 'flag'
  | 'toggleRead'
  | 'compose'
  | 'search'
  | 'help'
  | 'close';

export interface Shortcut {
  keys: string;
  action: ShortcutAction;
  label: string;
}

/** Shown in the help sheet, in this order. */
export const SHORTCUTS: readonly Shortcut[] = [
  { keys: 'j', action: 'next', label: 'Next message' },
  { keys: 'k', action: 'previous', label: 'Previous message' },
  { keys: 'Enter', action: 'open', label: 'Open message' },
  { keys: 'r', action: 'reply', label: 'Reply' },
  { keys: 'a', action: 'replyAll', label: 'Reply all' },
  { keys: 'f', action: 'forward', label: 'Forward' },
  { keys: 'e', action: 'archive', label: 'Archive' },
  { keys: '#', action: 'delete', label: 'Delete' },
  { keys: 's', action: 'flag', label: 'Flag or unflag' },
  { keys: 'u', action: 'toggleRead', label: 'Mark read or unread' },
  { keys: 'c', action: 'compose', label: 'New message' },
  { keys: '/', action: 'search', label: 'Search' },
  { keys: '?', action: 'help', label: 'Show shortcuts' },
  { keys: 'Esc', action: 'close', label: 'Close' },
];

const BY_KEY: ReadonlyMap<string, ShortcutAction> = new Map([
  ['j', 'next'],
  ['k', 'previous'],
  ['Enter', 'open'],
  ['r', 'reply'],
  ['a', 'replyAll'],
  ['f', 'forward'],
  ['e', 'archive'],
  ['#', 'delete'],
  ['s', 'flag'],
  ['u', 'toggleRead'],
  ['c', 'compose'],
  ['/', 'search'],
  ['?', 'help'],
  ['Escape', 'close'],
]);

export interface KeyInput {
  key: string;
  ctrlKey: boolean;
  metaKey: boolean;
  altKey: boolean;
  /** The element that has focus when the key is pressed. */
  target: EventTarget | null;
}

function isTyping(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target.isContentEditable) return true;
  const tag = target.tagName;
  if (tag === 'TEXTAREA' || tag === 'SELECT') return true;
  if (tag !== 'INPUT') return false;
  const type = (target as HTMLInputElement).type;
  return !['button', 'checkbox', 'radio', 'submit', 'reset'].includes(type);
}

function isActivatable(target: EventTarget | null): boolean {
  return target instanceof HTMLElement && ['BUTTON', 'A', 'SUMMARY'].includes(target.tagName);
}

/**
 * The action for a key press, or null. Typing in a field never triggers a
 * shortcut (except Esc), modifier combinations are left to the system, and
 * Enter on a focused button keeps its normal meaning.
 */
export function shortcutFor(input: KeyInput): ShortcutAction | null {
  if (input.ctrlKey || input.metaKey || input.altKey) return null;
  const action = BY_KEY.get(input.key) ?? null;
  if (action === null) return null;
  if (action === 'close') return action;
  if (isTyping(input.target)) return null;
  if (action === 'open' && isActivatable(input.target)) return null;
  return action;
}
