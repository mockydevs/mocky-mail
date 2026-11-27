import { useState } from 'react';

import type { Account } from '../../api/types';
import { usePhoneBack } from '../../app/usePhone';
import { Modal } from '../../ui/Modal';

/** The select's value for "this device" (accountId null) and for everyone ('*'). */
const DEVICE = 'device';
const EVERYONE = '*';

export function bookValue(accountId: string | null): string {
  return accountId ?? DEVICE;
}

export function bookFromValue(value: string): string | null {
  return value === DEVICE ? null : value;
}

/** An account in a list of choices: its name and address, "Work (you@work.example)". */
export function accountLabel(account: Account): string {
  return account.displayName && account.displayName !== account.email
    ? `${account.displayName} (${account.email})`
    : account.email;
}

/** Address books to choose from: this device, each account, and (for export) everyone. */
export function AddressBookOptions({ accounts, everyone = false }: { accounts: readonly Account[]; everyone?: boolean }) {
  return (
    <>
      {everyone && <option value={EVERYONE}>All contacts</option>}
      <option value={DEVICE}>This device only</option>
      {accounts.map((a) => (
        <option key={a.id} value={a.id}>
          {accountLabel(a)}
        </option>
      ))}
    </>
  );
}

/**
 * Asks where contacts go (import) or which ones to save (export) before the
 * native file picker opens.
 */
export function AddressBookPicker({
  mode,
  title,
  accounts,
  initial = null,
  onPick,
  onClose,
}: {
  mode: 'import' | 'export';
  title: string;
  accounts: readonly Account[];
  /** Preselected: an account id, or null for this device. */
  initial?: string | null;
  onPick: (accountId: string | null) => void;
  onClose: () => void;
}) {
  usePhoneBack(1, onClose);
  const [value, setValue] = useState(mode === 'export' ? EVERYONE : bookValue(initial ?? accounts[0]?.id ?? null));
  return (
    <Modal
      title={title}
      description={
        mode === 'import'
          ? 'Choose where the contacts go. You pick the .vcf file next; contacts on an account sync to its server.'
          : 'Choose which contacts to save in one .vcf file. You pick where it goes next.'
      }
      onClose={onClose}
      size="small"
    >
      <label className="form-field">
        <span className="form-field__label">{mode === 'import' ? 'Add them to' : 'Contacts from'}</span>
        <select
          className="field"
          value={value}
          onChange={(e) => {
            setValue(e.target.value);
          }}
        >
          <AddressBookOptions accounts={accounts} everyone={mode === 'export'} />
        </select>
      </label>
      <div className="modal__actions">
        <button type="button" className="btn btn--secondary" onClick={onClose}>
          Cancel
        </button>
        <button
          type="button"
          className="btn btn--primary"
          onClick={() => {
            onPick(bookFromValue(value));
          }}
        >
          {mode === 'import' ? 'Choose file…' : 'Save…'}
        </button>
      </div>
    </Modal>
  );
}
