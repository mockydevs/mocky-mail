import type { Mark } from '@tiptap/pm/model';
import type { EditorState } from '@tiptap/pm/state';
import type { EditorView } from '@tiptap/pm/view';

/** A painter copies how text looks, not where it links to. */
const KEEP = new Set(['link']);

/** The marks at the cursor, or on the first text of the selection. */
export function marksAt(state: EditorState): readonly Mark[] {
  const { selection, storedMarks } = state;
  if (storedMarks) return storedMarks.filter((m) => !KEEP.has(m.type.name));
  const found: (readonly Mark[])[] = [];
  if (!selection.empty) {
    state.doc.nodesBetween(selection.from, selection.to, (node) => {
      if (found.length === 0 && node.isText) found.push(node.marks);
      return found.length === 0;
    });
  }
  return (found[0] ?? selection.$from.marks()).filter((m) => !KEEP.has(m.type.name));
}

/** Give the selected text exactly these marks (links stay). False when nothing is selected. */
export function paintSelection(view: EditorView, marks: readonly Mark[]): boolean {
  const { from, to, empty } = view.state.selection;
  if (empty) return false;
  const tr = view.state.tr;
  for (const type of Object.values(view.state.schema.marks)) {
    if (!KEEP.has(type.name)) tr.removeMark(from, to, type);
  }
  for (const mark of marks) tr.addMark(from, to, mark);
  view.dispatch(tr);
  return true;
}
