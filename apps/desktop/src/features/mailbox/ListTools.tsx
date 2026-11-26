import type { ListSort, Settings } from '../../api/types';
import { MenuButton, MenuGroup, MenuItem } from '../../ui/Menu';
import type { GroupBy, ListOrder } from './useListView';

const SORTS: readonly [ListSort, string][] = [
  ['date', 'Date'],
  ['from', 'From'],
  ['subject', 'Subject'],
  ['size', 'Size'],
];

const GROUPS: readonly [GroupBy, string][] = [
  ['date', 'Date'],
  ['from', 'Sender'],
  ['importance', 'Importance'],
];

/** The two directions for a sort, in words that fit it. */
export function directionLabels(sort: ListSort): { descending: string; ascending: string } {
  switch (sort) {
    case 'date':
      return { descending: 'Newest first', ascending: 'Oldest first' };
    case 'size':
      return { descending: 'Largest first', ascending: 'Smallest first' };
    case 'from':
    case 'subject':
      return { descending: 'Z to A', ascending: 'A to Z' };
  }
}

const PANES: readonly [Settings['readingPane'], string][] = [
  ['right', 'Right'],
  ['bottom', 'Bottom'],
  ['off', 'Off'],
];

interface Props {
  order: ListOrder;
  onOrder: (order: ListOrder) => void;
  /** Computers: where the open message shows; phones have no choice. */
  pane?: Settings['readingPane'] | null;
  onPane?: (pane: Settings['readingPane']) => void;
}

/** Sort and group the list, and (computers) move the reading pane. */
export function ListTools({ order, onOrder, pane = null, onPane }: Props) {
  const directions = directionLabels(order.sort);
  const sortName = SORTS.find(([s]) => s === order.sort)?.[1] ?? 'Date';
  return (
    <>
      <MenuButton
        label={`Sort and group: ${sortName}, ${order.descending ? directions.descending : directions.ascending}`}
        icon="sort"
        title="Sort and group"
      >
        <MenuGroup label="Sort by">
          {SORTS.map(([sort, label]) => (
            <MenuItem
              key={sort}
              kind="radio"
              checked={order.sort === sort}
              onSelect={() => {
                // Dates and sizes read best biggest first, names from A.
                onOrder({ ...order, sort, descending: sort === 'date' || sort === 'size' });
              }}
            >
              {label}
            </MenuItem>
          ))}
        </MenuGroup>
        <MenuGroup label="Order">
          <MenuItem
            kind="radio"
            checked={order.descending}
            onSelect={() => {
              onOrder({ ...order, descending: true });
            }}
          >
            {directions.descending}
          </MenuItem>
          <MenuItem
            kind="radio"
            checked={!order.descending}
            onSelect={() => {
              onOrder({ ...order, descending: false });
            }}
          >
            {directions.ascending}
          </MenuItem>
        </MenuGroup>
        <MenuGroup label="Group by">
          {GROUPS.map(([groupBy, label]) => (
            <MenuItem
              key={groupBy}
              kind="radio"
              checked={order.groupBy === groupBy}
              onSelect={() => {
                onOrder({ ...order, groupBy });
              }}
            >
              {label}
            </MenuItem>
          ))}
        </MenuGroup>
      </MenuButton>
      {pane && onPane && (
        <MenuButton label={`Reading pane: ${PANES.find(([p]) => p === pane)?.[1] ?? 'Right'}`} icon="layout" title="Reading pane">
          <MenuGroup label="Reading pane">
            {PANES.map(([value, label]) => (
              <MenuItem
                key={value}
                kind="radio"
                checked={pane === value}
                onSelect={() => {
                  onPane(value);
                }}
              >
                {label}
              </MenuItem>
            ))}
          </MenuGroup>
        </MenuButton>
      )}
    </>
  );
}
