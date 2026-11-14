import { useState, type ClipboardEvent, type KeyboardEvent } from 'react';

import { Icon } from '../../ui/Icon';
import { addressOf, isPlausibleAddress, splitRecipients } from './recipients';

interface Props {
  id: string;
  /** The field's name, for the chip list ("To recipients"). */
  label: string;
  /** The whole field as text, "A <a@x>, b@y". The draft keeps this form. */
  value: string;
  onChange: (value: string) => void;
  invalid: boolean;
  describedBy: string | undefined;
  autoFocus: boolean;
}

const join = (entries: readonly string[]) => entries.join(', ');

/** Whether quotes and angle brackets are closed, so a comma ends an entry. */
function balanced(text: string): boolean {
  let quoted = false;
  let angled = false;
  for (const ch of text) {
    if (ch === '"' && !angled) quoted = !quoted;
    else if (ch === '<' && !quoted) angled = true;
    else if (ch === '>' && !quoted) angled = false;
  }
  return !quoted && !angled;
}

/** "Name <a@x>" shows as Name; a bare address shows as itself. */
function chipLabel(entry: string): string {
  const angle = /^(.*?)<[^<>]*>\s*$/.exec(entry);
  const name = angle?.[1]?.trim().replace(/^"|"$/g, '');
  return name !== undefined && name !== '' ? name : entry;
}

/**
 * Recipients as removable chips, with a text box for the one being typed.
 * A comma, semicolon, Enter or leaving the field turns the text into a chip;
 * Backspace on an empty box takes the last chip back for editing.
 */
export function RecipientField({ id, label, value, onChange, invalid, describedBy, autoFocus }: Props) {
  const [text, setText] = useState('');
  const all = splitRecipients(value);
  const chips = text.trim() === '' ? all : all.slice(0, -1);

  const emit = (entries: readonly string[], typing: string) => {
    setText(typing);
    onChange(join(typing.trim() === '' ? entries : [...entries, typing.trim()]));
  };

  const type = (next: string) => {
    if (!balanced(next) || !/[,;\n]/.test(next)) {
      emit(chips, next);
      return;
    }
    const parts = splitRecipients(next);
    const ended = /[,;\n]\s*$/.test(next);
    const rest = ended ? '' : (parts.pop() ?? '');
    emit([...chips, ...parts], rest);
  };

  const commit = () => {
    if (text.trim() !== '') emit([...chips, text.trim()], '');
  };

  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'Enter' && !event.ctrlKey && !event.metaKey && text.trim() !== '') {
      event.preventDefault();
      commit();
    } else if (event.key === 'Backspace' && text === '' && chips.length > 0) {
      event.preventDefault();
      const last = chips.at(-1) ?? '';
      emit(chips.slice(0, -1), last);
    }
  };

  const onPaste = (event: ClipboardEvent<HTMLInputElement>) => {
    const pasted = event.clipboardData.getData('text');
    if (!/[,;\n]/.test(pasted)) return;
    event.preventDefault();
    type(`${text}${pasted},`);
  };

  return (
    <div className="recipients">
      {chips.length > 0 && (
        <ul className="recipients__list" aria-label={`${label} recipients`}>
          {chips.map((entry, index) => {
            const address = addressOf(entry);
            const ok = address !== null && isPlausibleAddress(address);
            return (
              // Entries are de-duplicated on send, not here, so the index keeps keys unique.
              <li key={`${index}-${entry}`} className={ok ? 'chip' : 'chip chip--bad'} title={entry}>
                <span className="chip__text">{chipLabel(entry)}</span>
                <button
                  type="button"
                  className="chip__remove"
                  aria-label={`Remove ${entry}`}
                  onClick={() => {
                    emit(
                      chips.filter((_, i) => i !== index),
                      text,
                    );
                  }}
                >
                  <Icon name="close" size={12} />
                </button>
              </li>
            );
          })}
        </ul>
      )}
      <input
        id={id}
        type="text"
        inputMode="email"
        autoComplete="email"
        className="compose__input recipients__input"
        value={text}
        aria-invalid={invalid ? true : undefined}
        aria-describedby={describedBy}
        // The first field of a view the user just opened.
        // eslint-disable-next-line jsx-a11y/no-autofocus
        autoFocus={autoFocus}
        onChange={(e) => {
          type(e.target.value);
        }}
        onKeyDown={onKeyDown}
        onPaste={onPaste}
        onBlur={commit}
      />
    </div>
  );
}
