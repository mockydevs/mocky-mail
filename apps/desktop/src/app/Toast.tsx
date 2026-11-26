import { Icon } from '../ui/Icon';
import type { Notice } from './useToasts';

export function Toast({ notice, onDismiss }: { notice: Notice; onDismiss: () => void }) {
  return (
    <div className={`toast toast--${notice.tone}`} role={notice.tone === 'error' ? 'alert' : 'status'}>
      <div>
        <p className="toast__title">{notice.title}</p>
        <p className="toast__text">{notice.text}</p>
      </div>
      {notice.action && (
        <button type="button" className="btn btn--secondary btn--small toast__action" onClick={notice.action.run}>
          {notice.action.label}
        </button>
      )}
      <button type="button" className="icon-btn" aria-label="Dismiss" onClick={onDismiss}>
        <Icon name="close" />
      </button>
    </div>
  );
}
