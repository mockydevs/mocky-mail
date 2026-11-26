import { useState, type CSSProperties } from 'react';

import type { Category, MessageListItem } from '../../api/types';
import { MenuButton, MenuItem, MenuSeparator } from '../../ui/Menu';
import { CategoryEditor } from './CategoryEditor';

/** Whether a category is on every message, some of them, or none. */
export function categoryState(rows: readonly MessageListItem[], name: string): boolean | 'mixed' {
  const on = rows.filter((r) => r.categories.some((c) => c.toLowerCase() === name.toLowerCase())).length;
  return on === 0 ? false : on === rows.length ? true : 'mixed';
}

interface Props {
  categories: readonly Category[];
  /** The messages being categorised. */
  rows: readonly MessageListItem[];
  /** Add the category to every message, or take it off every one. */
  onToggle: (name: string, add: boolean) => void;
  /** A category was made from "New category…": put it on the messages. */
  onCreated: (categories: Category[], name: string) => void;
  className?: string;
  /** Text on the button; icon only when missing. */
  text?: string;
}

/**
 * "Categorise": a tick per category (mixed when only some of the messages
 * have it), and "New category…" to make one and apply it straight away.
 */
export function CategoryMenu({ categories, rows, onToggle, onCreated, className, text }: Props) {
  const [creating, setCreating] = useState(false);
  return (
    <>
      <MenuButton label="Categorise" icon="tag" title="Categorise" {...(className ? { className } : {})} {...(text ? { text } : {})}>
        {categories.length === 0 && <p className="menu__note">No categories yet.</p>}
        {categories.map((c) => {
          const state = categoryState(rows, c.name);
          return (
            <MenuItem
              key={c.name}
              kind="check"
              checked={state}
              keepOpen
              onSelect={() => {
                onToggle(c.name, state !== true);
              }}
            >
              <span className="chip-dot" style={{ '--chip': c.color } as CSSProperties} aria-hidden="true" />
              {c.name}
            </MenuItem>
          );
        })}
        <MenuSeparator />
        <MenuItem
          icon="plus"
          onSelect={() => {
            setCreating(true);
          }}
        >
          New category…
        </MenuItem>
      </MenuButton>
      {creating && (
        <CategoryEditor
          title="New category"
          existing={categories}
          onClose={() => {
            setCreating(false);
          }}
          onSaved={(all, name) => {
            setCreating(false);
            onCreated(all, name);
          }}
        />
      )}
    </>
  );
}
