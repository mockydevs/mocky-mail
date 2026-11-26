import type { Category, FolderRole, MessageListItem } from '../../api/types';
import { Icon, type IconName } from '../../ui/Icon';
import { CategoryMenu } from '../categories/CategoryMenu';

export interface SelectionActions {
  onClear: () => void;
  onSelectAll: () => void;
  onRead: (read: boolean) => void;
  onFlag: (flagged: boolean) => void;
  onPin: (pinned: boolean) => void;
  onCategory: (name: string, add: boolean) => void;
  onCategoryCreated: (categories: Category[], name: string) => void;
  onMove: () => void;
  onCopy: () => void;
  onArchive: () => void;
  onDelete: () => void;
  onJunk: (junk: boolean) => void;
}

interface Props extends SelectionActions {
  rows: readonly MessageListItem[];
  /** Rows loaded in the list, for "Select all". */
  shown: number;
  role: FolderRole | null;
  categories: readonly Category[];
}

function Action({ icon, label, onClick, filled }: { icon: IconName; label: string; onClick: () => void; filled?: boolean }) {
  return (
    <button type="button" className="icon-btn" aria-label={label} title={label} onClick={onClick}>
      <Icon name={icon} filled={filled ?? false} />
    </button>
  );
}

/**
 * Replaces the list header while messages are ticked: how many, and what can
 * be done to all of them at once.
 */
export function SelectionBar(props: Props) {
  const { rows, role } = props;
  const anyUnread = rows.some((r) => r.unread);
  const allFlagged = rows.every((r) => r.flagged);
  const allPinned = rows.every((r) => r.pinned);
  const inBin = role === 'trash' || role === 'junk';
  const count = rows.length;
  return (
    <div className="selection-bar">
      <div className="selection-bar__top">
        <button type="button" className="icon-btn" aria-label="Clear selection" title="Clear selection (Esc)" onClick={props.onClear}>
          <Icon name="close" />
        </button>
        <p className="selection-bar__count" role="status">
          {count} selected
        </p>
        {count < props.shown && (
          <button type="button" className="btn btn--link btn--small" onClick={props.onSelectAll}>
            Select all {props.shown}
          </button>
        )}
      </div>
      <div className="selection-bar__actions" role="toolbar" aria-label="Selected messages">
        <Action
          icon={anyUnread ? 'envelopeOpen' : 'mail'}
          label={anyUnread ? 'Mark as read' : 'Mark as unread'}
          onClick={() => {
            props.onRead(anyUnread);
          }}
        />
        <Action
          icon="flag"
          filled={allFlagged}
          label={allFlagged ? 'Remove flag' : 'Flag'}
          onClick={() => {
            props.onFlag(!allFlagged);
          }}
        />
        <Action
          icon="pin"
          filled={allPinned}
          label={allPinned ? 'Unpin' : 'Pin'}
          onClick={() => {
            props.onPin(!allPinned);
          }}
        />
        <CategoryMenu
          categories={props.categories}
          rows={rows}
          onToggle={props.onCategory}
          onCreated={props.onCategoryCreated}
        />
        <Action icon="move" label="Move to…" onClick={props.onMove} />
        <Action icon="copy" label="Copy to…" onClick={props.onCopy} />
        {role !== 'archive' && !inBin && role !== 'drafts' && <Action icon="archive" label="Archive" onClick={props.onArchive} />}
        <Action icon="trash" label={inBin ? 'Delete forever' : 'Delete'} onClick={props.onDelete} />
        {role !== 'drafts' && role !== 'sent' && (
          <Action
            icon="junk"
            label={role === 'junk' ? 'Not junk' : 'Junk'}
            onClick={() => {
              props.onJunk(role !== 'junk');
            }}
          />
        )}
      </div>
    </div>
  );
}
