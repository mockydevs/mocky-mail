import { Modal } from '../../ui/Modal';
import { SHORTCUTS } from './shortcuts';

export function HelpSheet({ onClose }: { onClose: () => void }) {
  return (
    <Modal title="Keyboard shortcuts" onClose={onClose}>
      <dl className="shortcuts">
        {SHORTCUTS.map((s) => (
          <div key={s.action} className="shortcuts__row">
            <dt>
              <kbd>{s.keys}</kbd>
            </dt>
            <dd>{s.label}</dd>
          </div>
        ))}
      </dl>
      <div className="modal__actions">
        <button type="button" className="btn btn--primary" onClick={onClose}>
          Done
        </button>
      </div>
    </Modal>
  );
}
