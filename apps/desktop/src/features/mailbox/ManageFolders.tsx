import { useId, useState, type SyntheticEvent } from 'react';

import { useMailApi } from '../../api/MailApiContext';
import { toAppError } from '../../api/errors';
import type { Account, AppError, Folder } from '../../api/types';
import { Icon } from '../../ui/Icon';
import { ConfirmDialog, Modal } from '../../ui/Modal';

interface Props {
  account: Account;
  /** The account's folders, as the sidebar shows them. */
  folders: readonly Folder[];
  onClose: () => void;
}

/** "Clients / Orders" for nested folders, so the list reads like the sidebar. */
function fullName(folder: Folder, all: readonly Folder[]): string {
  const d = folder.delimiter;
  if (!d) return folder.name;
  const parts: string[] = [];
  let path = folder.path;
  while (path) {
    const match = all.find((f) => f.path === path);
    parts.unshift(match?.name ?? path.slice(path.lastIndexOf(d) + 1));
    const cut = path.lastIndexOf(d);
    path = cut > 0 ? path.slice(0, cut) : '';
  }
  return parts.join(' / ');
}

/**
 * Create, rename and delete the account's own folders on the server.
 * Built-in folders (Inbox, Sent, Drafts, Archive, Spam, Trash) are shown
 * only as places to put a new folder in.
 */
export function ManageFolders({ account, folders, onClose }: Props) {
  const api = useMailApi();
  const id = useId();
  const own = folders
    .filter((f) => f.role === 'other')
    .map((f) => ({ folder: f, label: fullName(f, folders) }))
    .sort((a, b) => a.label.localeCompare(b.label));
  const [name, setName] = useState('');
  const [parentId, setParentId] = useState('');
  const [renaming, setRenaming] = useState<{ id: string; name: string } | null>(null);
  const [deleting, setDeleting] = useState<Folder | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<AppError | null>(null);
  const [done, setDone] = useState<string | null>(null);

  const run = (work: () => Promise<void>, success: string, after?: () => void) => {
    setBusy(true);
    setError(null);
    setDone(null);
    work().then(
      () => {
        setBusy(false);
        setDone(success);
        after?.();
      },
      (e: unknown) => {
        setBusy(false);
        setError(toAppError(e));
      },
    );
  };

  const create = (event: SyntheticEvent) => {
    event.preventDefault();
    const typed = name.trim();
    run(
      () => api.createFolder(account.id, typed, parentId || null),
      `Folder "${typed}" created.`,
      () => {
        setName('');
      },
    );
  };

  return (
    <Modal title="Manage folders" description={`Folders for ${account.email}, kept on the mail server.`} onClose={onClose}>
      <form className="folder-manage__new" aria-labelledby={`${id}-new`} onSubmit={create} noValidate>
        <h3 id={`${id}-new`} className="folder-manage__title">
          New folder
        </h3>
        <label className="form-field">
          <span className="form-field__label">Name</span>
          <input
            className="field"
            value={name}
            maxLength={100}
            onChange={(e) => {
              setName(e.target.value);
            }}
          />
        </label>
        <label className="form-field">
          <span className="form-field__label">Put it in</span>
          <select
            className="field"
            value={parentId}
            onChange={(e) => {
              setParentId(e.target.value);
            }}
          >
            <option value="">The top level</option>
            {own.map(({ folder, label }) => (
              <option key={folder.id} value={folder.id}>
                {label}
              </option>
            ))}
          </select>
        </label>
        <button type="submit" className="btn btn--primary" disabled={busy || name.trim() === ''}>
          Create folder
        </button>
      </form>

      <h3 className="folder-manage__title" id={`${id}-yours`}>
        Your folders
      </h3>
      {own.length === 0 ? (
        <p className="muted">You haven't made any folders yet.</p>
      ) : (
        <ul className="folder-manage__list" aria-labelledby={`${id}-yours`}>
          {own.map(({ folder, label }) => (
            <li key={folder.id} className="folder-manage__row">
              {renaming?.id === folder.id ? (
                <form
                  className="folder-manage__rename"
                  onSubmit={(event) => {
                    event.preventDefault();
                    const next = renaming.name.trim();
                    run(() => api.renameFolder(folder.id, next), `Renamed to "${next}".`, () => {
                      setRenaming(null);
                    });
                  }}
                >
                  <input
                    className="field"
                    aria-label={`New name for ${label}`}
                    value={renaming.name}
                    maxLength={100}
                    // eslint-disable-next-line jsx-a11y/no-autofocus
                    autoFocus
                    onChange={(e) => {
                      setRenaming({ id: folder.id, name: e.target.value });
                    }}
                  />
                  <button type="submit" className="btn btn--primary btn--small" disabled={busy}>
                    Save
                  </button>
                  <button
                    type="button"
                    className="btn btn--link"
                    onClick={() => {
                      setRenaming(null);
                    }}
                  >
                    Cancel
                  </button>
                </form>
              ) : (
                <>
                  <span className="folder-manage__name">
                    {label}
                    <span className="muted small"> · {folder.total} emails</span>
                  </span>
                  <button
                    type="button"
                    className={`icon-btn folder-manage__star${folder.favorite ? ' folder-manage__star--on' : ''}`}
                    aria-label={folder.favorite ? `Remove ${label} from Favourites` : `Add ${label} to Favourites`}
                    title={folder.favorite ? 'In Favourites' : 'Add to Favourites'}
                    aria-pressed={folder.favorite}
                    disabled={busy}
                    onClick={() => {
                      run(
                        () => api.setFolderFavorite(folder.id, !folder.favorite),
                        folder.favorite ? `${label} removed from Favourites.` : `${label} added to Favourites.`,
                      );
                    }}
                  >
                    <Icon name="star" filled={folder.favorite} />
                  </button>
                  <button
                    type="button"
                    className="btn btn--link"
                    aria-label={`Rename ${label}`}
                    disabled={busy}
                    onClick={() => {
                      setRenaming({ id: folder.id, name: folder.name });
                    }}
                  >
                    Rename
                  </button>
                  <button
                    type="button"
                    className="btn btn--link btn--danger-text"
                    aria-label={`Delete ${label}`}
                    disabled={busy}
                    onClick={() => {
                      setDeleting(folder);
                    }}
                  >
                    Delete
                  </button>
                </>
              )}
            </li>
          ))}
        </ul>
      )}

      {busy && (
        <p className="form-hint" role="status">
          Working on the server…
        </p>
      )}
      {error && (
        <p className="form-error" role="alert">
          {error.message}
        </p>
      )}
      {done && !busy && (
        <p className="form-hint" role="status">
          {done}
        </p>
      )}
      <div className="modal__actions">
        <button type="button" className="btn btn--secondary" onClick={onClose}>
          Done
        </button>
      </div>

      {deleting && (
        <ConfirmDialog
          title={`Delete ${deleting.name}?`}
          description={
            deleting.total > 0
              ? `The folder and the ${String(deleting.total)} emails in it are deleted from the server. This can't be undone.`
              : "The folder is deleted from the server. This can't be undone."
          }
          confirmLabel="Delete folder"
          danger
          onCancel={() => {
            setDeleting(null);
          }}
          onConfirm={() => {
            const target = deleting;
            setDeleting(null);
            run(() => api.deleteFolder(target.id), `Folder "${target.name}" deleted.`);
          }}
        />
      )}
    </Modal>
  );
}
