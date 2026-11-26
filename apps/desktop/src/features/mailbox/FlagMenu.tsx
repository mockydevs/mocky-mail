import { useState, type SyntheticEvent } from 'react';

import { MenuButton, MenuItem, MenuSeparator } from '../../ui/Menu';
import { Modal } from '../../ui/Modal';
import { dateValue, DUE_CHOICES, dueFor, dueFromDate } from './flagDue';
import type { FlagValue } from './useMessageActions';

interface Props {
  flagged: boolean;
  due: number | null;
  now: number;
  onFlag: (flag: FlagValue) => void;
  className?: string;
}

/** The small menu beside the flag button: flag with a due date, without one, or clear it. */
export function FlagMenu({ flagged, due, now, onFlag, className }: Props) {
  const [picking, setPicking] = useState(false);
  return (
    <>
      <MenuButton label="Flag options" icon="clock" title="Flag with a due date" {...(className ? { className } : {})}>
        {DUE_CHOICES.map((choice) => (
          <MenuItem
            key={choice.id}
            onSelect={() => {
              onFlag({ flagged: true, due: dueFor(choice.id, now) });
            }}
          >
            {choice.label}
          </MenuItem>
        ))}
        <MenuItem
          onSelect={() => {
            setPicking(true);
          }}
        >
          Pick a date…
        </MenuItem>
        <MenuItem
          onSelect={() => {
            onFlag({ flagged: true, due: null });
          }}
        >
          No date
        </MenuItem>
        <MenuSeparator />
        <MenuItem
          disabled={!flagged}
          onSelect={() => {
            onFlag({ flagged: false, due: null });
          }}
        >
          Clear flag
        </MenuItem>
      </MenuButton>
      {picking && (
        <DueDateDialog
          initial={due ?? dueFor('tomorrow', now)}
          onClose={() => {
            setPicking(false);
          }}
          onPick={(value) => {
            setPicking(false);
            onFlag({ flagged: true, due: value });
          }}
        />
      )}
    </>
  );
}

function DueDateDialog({ initial, onClose, onPick }: { initial: number; onClose: () => void; onPick: (due: number) => void }) {
  const [value, setValue] = useState(dateValue(initial));
  const due = dueFromDate(value);
  const submit = (event: SyntheticEvent) => {
    event.preventDefault();
    if (due !== null) onPick(due);
  };
  return (
    <Modal title="Flag with a due date" description="You get a reminder at 17:00 that day." onClose={onClose} size="small">
      <form onSubmit={submit} noValidate>
        <label className="form-field">
          <span className="form-field__label">Due</span>
          <input
            type="date"
            className="field"
            value={value}
            onChange={(e) => {
              setValue(e.target.value);
            }}
          />
        </label>
        <div className="modal__actions">
          <button type="button" className="btn btn--secondary" onClick={onClose}>
            Cancel
          </button>
          <button type="submit" className="btn btn--primary" disabled={due === null}>
            Flag
          </button>
        </div>
      </form>
    </Modal>
  );
}
