import type { FolderRole, MessageListItem } from '../../api/types';
import { Icon, type IconName } from '../../ui/Icon';

interface Props {
  item: MessageListItem;
  role: FolderRole | null;
  onPin: () => void;
  onFlag: () => void;
  onRead: () => void;
  onJunk: (junk: boolean) => void;
  onArchive: () => void;
  onDelete: () => void;
}

function Action({ icon, label, filled, onClick }: { icon: IconName; label: string; filled?: boolean; onClick: () => void }) {
  return (
    <button type="button" className="icon-btn icon-btn--small" aria-label={label} title={label} onClick={onClick}>
      <Icon name={icon} size={16} filled={filled ?? false} />
    </button>
  );
}

/** Quick actions shown over the row under the pointer (computers). */
export function RowActions({ item, role, onPin, onFlag, onRead, onJunk, onArchive, onDelete }: Props) {
  const inBin = role === 'trash' || role === 'junk';
  return (
    <>
      {role !== 'drafts' && <Action icon="pin" label={item.pinned ? 'Unpin' : 'Pin'} filled={item.pinned} onClick={onPin} />}
      {role !== 'drafts' && (
        <Action icon="flag" label={item.flagged ? 'Remove flag' : 'Flag'} filled={item.flagged} onClick={onFlag} />
      )}
      <Action icon={item.unread ? 'envelopeOpen' : 'mail'} label={item.unread ? 'Mark as read' : 'Mark as unread'} onClick={onRead} />
      {role !== 'drafts' && role !== 'sent' && (
        <Action
          icon="junk"
          label={role === 'junk' ? 'Not junk' : 'Junk'}
          onClick={() => {
            onJunk(role !== 'junk');
          }}
        />
      )}
      {role !== 'archive' && !inBin && role !== 'drafts' && <Action icon="archive" label="Archive" onClick={onArchive} />}
      <Action icon="trash" label={inBin ? 'Delete forever' : 'Delete'} onClick={onDelete} />
    </>
  );
}
