import { useId, useState, type CSSProperties } from 'react';

import { useMailApi } from '../../api/MailApiContext';
import { toAppError } from '../../api/errors';
import type { AppError, Category } from '../../api/types';
import { Icon } from '../../ui/Icon';
import { ConfirmDialog } from '../../ui/Modal';
import { CategoryEditor } from './CategoryEditor';

interface Props {
  categories: readonly Category[];
  onChange: (categories: Category[]) => void;
}

/** Settings › Categories: add, rename, recolour and delete colour categories. */
export function CategoryManager({ categories, onChange }: Props) {
  const api = useMailApi();
  const id = useId();
  const [editing, setEditing] = useState<Category | 'new' | null>(null);
  const [deleting, setDeleting] = useState<Category | null>(null);
  const [error, setError] = useState<AppError | null>(null);
  const [done, setDone] = useState<string | null>(null);

  return (
    <section className="settings__group settings__group--wide" aria-labelledby={`${id}-title`}>
      <h2 id={`${id}-title`} className="settings__legend">
        Categories
      </h2>
      <p className="form-hint">Colour labels for messages, shared by all your accounts.</p>
      {categories.length === 0 ? (
        <p className="muted">No categories yet.</p>
      ) : (
        <ul className="category-list" aria-labelledby={`${id}-title`}>
          {categories.map((c) => (
            <li key={c.name} className="category-list__row">
              <span className="chip" style={{ '--chip': c.color } as CSSProperties}>
                {c.name}
              </span>
              <span className="category-list__hex muted small">{c.color}</span>
              <button
                type="button"
                className="btn btn--link"
                aria-label={`Edit ${c.name}`}
                onClick={() => {
                  setDone(null);
                  setEditing(c);
                }}
              >
                Edit
              </button>
              <button
                type="button"
                className="btn btn--link btn--danger-text"
                aria-label={`Delete ${c.name}`}
                onClick={() => {
                  setDone(null);
                  setDeleting(c);
                }}
              >
                Delete
              </button>
            </li>
          ))}
        </ul>
      )}
      <button
        type="button"
        className="btn btn--secondary"
        onClick={() => {
          setDone(null);
          setEditing('new');
        }}
      >
        <Icon name="plus" />
        New category
      </button>
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
        <CategoryEditor
          title={editing === 'new' ? 'New category' : `Edit ${editing.name}`}
          {...(editing === 'new' ? {} : { initial: editing })}
          existing={categories}
          onClose={() => {
            setEditing(null);
          }}
          onSaved={(all, name) => {
            setEditing(null);
            setError(null);
            setDone(editing === 'new' ? `Category "${name}" added.` : `Category "${name}" saved.`);
            onChange(all);
          }}
        />
      )}
      {deleting && (
        <ConfirmDialog
          title={`Delete ${deleting.name}?`}
          description="It is taken off every message that has it. The messages themselves stay."
          confirmLabel="Delete category"
          danger
          onCancel={() => {
            setDeleting(null);
          }}
          onConfirm={() => {
            const target = deleting;
            setDeleting(null);
            api.deleteCategory(target.name).then(
              (all) => {
                setError(null);
                setDone(`Category "${target.name}" deleted.`);
                onChange(all);
              },
              (e: unknown) => {
                setError(toAppError(e));
              },
            );
          }}
        />
      )}
    </section>
  );
}
