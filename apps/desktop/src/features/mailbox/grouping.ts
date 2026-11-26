import type { Importance, MessageListItem } from '../../api/types';
import { dayGroup } from '../../lib/format';
import type { GroupBy } from './useListView';

export interface RowGroup {
  label: string;
  items: MessageListItem[];
}

const IMPORTANCE_GROUPS: readonly [Importance, string][] = [
  ['high', 'High importance'],
  ['normal', 'Normal importance'],
  ['low', 'Low importance'],
];

/** Who a row is from (or, for Sent and Drafts, to): the sender heading. */
export function senderOf(item: MessageListItem, outgoing: boolean): string {
  if (outgoing) {
    const first = item.to[0];
    return first ? `To ${first.name ?? first.address}` : 'No recipients';
  }
  return item.from ? (item.from.name ?? item.from.address) : 'Unknown sender';
}

/**
 * Put the loaded rows under headings. The order inside each group is the
 * list's sort; groups come in the order their first row appears, except
 * importance, which is always high, normal, low. Pinned rows at the top of a
 * folder get their own heading when grouping by date.
 */
export function groupRows(items: readonly MessageListItem[], groupBy: GroupBy, now: number, outgoing = false): RowGroup[] {
  if (groupBy === 'importance') {
    return IMPORTANCE_GROUPS.map(([importance, label]) => ({
      label,
      items: items.filter((m) => m.importance === importance),
    })).filter((g) => g.items.length > 0);
  }
  const groups: RowGroup[] = [];
  const byLabel = new Map<string, RowGroup>();
  let leadingPins = true;
  for (const item of items) {
    leadingPins = leadingPins && item.pinned;
    const label =
      groupBy === 'from'
        ? senderOf(item, outgoing)
        : leadingPins
          ? 'Pinned'
          : dayGroup(item.date, now);
    let group = byLabel.get(label);
    if (!group) {
      group = { label, items: [] };
      byLabel.set(label, group);
      groups.push(group);
    }
    group.items.push(item);
  }
  return groups;
}
