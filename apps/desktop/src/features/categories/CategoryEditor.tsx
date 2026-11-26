import { useId, useState, type CSSProperties, type SyntheticEvent } from 'react';

import { useMailApi } from '../../api/MailApiContext';
import { toAppError } from '../../api/errors';
import type { AppError, Category } from '../../api/types';
import { Modal } from '../../ui/Modal';

/** The colours offered for a category; any other `#rrggbb` can be typed. */
export const CATEGORY_PALETTE: readonly { color: string; name: string }[] = [
  { color: '#b42318', name: 'Red' },
  { color: '#c2410c', name: 'Orange' },
  { color: '#a16207', name: 'Yellow' },
  { color: '#1e6b3a', name: 'Green' },
  { color: '#0f7b6c', name: 'Teal' },
  { color: '#0369a1', name: 'Sky' },
  { color: '#2450c8', name: 'Blue' },
  { color: '#8a3fb0', name: 'Purple' },
  { color: '#a1356b', name: 'Pink' },
  { color: '#4b5563', name: 'Grey' },
];

const HEX = /^#[0-9a-f]{6}$/i;

/** `#rrggbb` from what was typed ("2450c8", "#2450C8"), or null. */
export function cleanHex(value: string): string | null {
  const v = value.trim();
  const withHash = v.startsWith('#') ? v : `#${v}`;
  return HEX.test(withHash) ? withHash.toLowerCase() : null;
}

interface Props {
  title: string;
  /** The category being changed; a new one when missing. */
  initial?: Category;
  existing: readonly Category[];
  onClose: () => void;
  /** Every category after saving, and the saved one's name. */
  onSaved: (categories: Category[], name: string) => void;
}

/** Name and colour for a new category, or a rename and recolour of one. */
export function CategoryEditor({ title, initial, existing, onClose, onSaved }: Props) {
  const api = useMailApi();
  const id = useId();
  const [name, setName] = useState(initial?.name ?? '');
  const firstFree = CATEGORY_PALETTE.find((p) => !existing.some((c) => c.color.toLowerCase() === p.color))?.color;
  const [color, setColor] = useState(initial?.color.toLowerCase() ?? firstFree ?? CATEGORY_PALETTE[0]?.color ?? '#2450c8');
  const [hex, setHex] = useState(color);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<AppError | null>(null);
  const custom = !CATEGORY_PALETTE.some((p) => p.color === color);

  const save = (event: SyntheticEvent) => {
    event.preventDefault();
    const typed = name.trim();
    if (!typed) {
      setError({ kind: 'invalid', message: 'Type a name for the category.' });
      return;
    }
    setBusy(true);
    setError(null);
    api.saveCategory({ name: typed, color }, initial?.name ?? null).then(
      (all) => {
        setBusy(false);
        onSaved(all, typed);
      },
      (e: unknown) => {
        setBusy(false);
        setError(toAppError(e));
      },
    );
  };

  return (
    <Modal title={title} onClose={onClose} size="small">
      <form className="category-editor" onSubmit={save} noValidate>
        <label className="form-field">
          <span className="form-field__label">Name</span>
          <input
            className="field"
            value={name}
            maxLength={40}
            onChange={(e) => {
              setName(e.target.value);
            }}
          />
        </label>
        <fieldset className="category-editor__colours">
          <legend className="form-field__label">Colour</legend>
          <div className="swatches">
            {CATEGORY_PALETTE.map((p) => (
              <label key={p.color} className="swatch" style={{ '--chip': p.color } as CSSProperties} title={p.name}>
                <input
                  type="radio"
                  name={`${id}-colour`}
                  className="visually-hidden"
                  checked={color === p.color}
                  aria-label={p.name}
                  onChange={() => {
                    setColor(p.color);
                    setHex(p.color);
                  }}
                />
                <span className="swatch__dot" aria-hidden="true" />
              </label>
            ))}
          </div>
          <label className="form-field category-editor__hex">
            <span className="form-field__label">Or any colour (hex)</span>
            <span className="category-editor__hex-row">
              <span
                className={`swatch__dot${custom ? ' swatch__dot--on' : ''}`}
                style={{ '--chip': color } as CSSProperties}
                aria-hidden="true"
              />
              <input
                className="field"
                value={hex}
                maxLength={7}
                spellCheck={false}
                aria-invalid={cleanHex(hex) === null}
                onChange={(e) => {
                  setHex(e.target.value);
                  const clean = cleanHex(e.target.value);
                  if (clean) setColor(clean);
                }}
              />
            </span>
          </label>
        </fieldset>
        {error && (
          <p className="form-error" role="alert">
            {error.message}
          </p>
        )}
        <div className="modal__actions">
          <button type="button" className="btn btn--secondary" onClick={onClose}>
            Cancel
          </button>
          <button type="submit" className="btn btn--primary" disabled={busy}>
            {initial ? 'Save' : 'Create'}
          </button>
        </div>
      </form>
    </Modal>
  );
}
