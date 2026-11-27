import { Extension } from '@tiptap/core';
import type { Node as PmNode } from '@tiptap/pm/model';
import { Plugin, PluginKey, TextSelection, type EditorState, type Transaction } from '@tiptap/pm/state';
import { Decoration, DecorationSet, type EditorView } from '@tiptap/pm/view';

export interface Match {
  from: number;
  to: number;
}

export interface FindState {
  query: string;
  matchCase: boolean;
  /** Index of the current match, or -1. */
  current: number;
  matches: Match[];
  decorations: DecorationSet;
}

export const findKey = new PluginKey<FindState>('mockyFind');

/** Every place `query` appears in the text, one paragraph at a time. */
export function findMatches(doc: PmNode, query: string, matchCase: boolean): Match[] {
  if (query === '') return [];
  const needle = matchCase ? query : query.toLowerCase();
  const out: Match[] = [];
  doc.descendants((node, pos) => {
    if (!node.isTextblock) return true;
    let text = '';
    const at: number[] = [];
    node.forEach((child, offset) => {
      if (child.isText && child.text !== undefined) {
        for (let i = 0; i < child.text.length; i++) at.push(pos + 1 + offset + i);
        text += child.text;
      } else {
        // A line break or similar: a match never spans it.
        at.push(-1);
        text += '￼';
      }
    });
    const hay = matchCase ? text : text.toLowerCase();
    let index = hay.indexOf(needle);
    while (index !== -1) {
      const span = at.slice(index, index + needle.length);
      const first = span[0];
      const last = span[span.length - 1];
      if (first !== undefined && last !== undefined && !span.includes(-1)) out.push({ from: first, to: last + 1 });
      index = hay.indexOf(needle, index + needle.length);
    }
    return false;
  });
  return out;
}

function build(doc: PmNode, query: string, matchCase: boolean, current: number): FindState {
  const matches = findMatches(doc, query, matchCase);
  // Past the last one (it was just replaced): round to the first.
  const index = matches.length === 0 ? -1 : current >= matches.length ? 0 : Math.max(current, 0);
  const decorations = DecorationSet.create(
    doc,
    matches.map((m, i) => Decoration.inline(m.from, m.to, { class: i === index ? 'rte-find rte-find--current' : 'rte-find' })),
  );
  return { query, matchCase, current: index, matches, decorations };
}

type Meta = Partial<Pick<FindState, 'query' | 'matchCase' | 'current'>>;

export const FindReplace = Extension.create({
  name: 'mockyFindReplace',
  addProseMirrorPlugins() {
    return [
      new Plugin<FindState>({
        key: findKey,
        state: {
          init: (_, state) => build(state.doc, '', false, -1),
          apply(tr, old) {
            const meta = tr.getMeta(findKey) as Meta | undefined;
            if (!meta && !tr.docChanged) return old;
            const next = { query: old.query, matchCase: old.matchCase, current: old.current, ...meta };
            if (next.query === '' && old.query === '') return old;
            return build(tr.doc, next.query, next.matchCase, next.current);
          },
        },
        props: {
          decorations: (state) => findKey.getState(state)?.decorations ?? null,
        },
      }),
    ];
  },
});

export function findState(state: EditorState): FindState | undefined {
  return findKey.getState(state);
}

function select(tr: Transaction, match: Match | undefined): Transaction {
  if (!match) return tr;
  return tr.setSelection(TextSelection.create(tr.doc, match.from, match.to)).scrollIntoView();
}

/** Look for `query`; the current match is the first one at or after the cursor. */
export function setSearch(view: EditorView, query: string, matchCase: boolean): void {
  const matches = findMatches(view.state.doc, query, matchCase);
  const cursor = view.state.selection.from;
  const after = matches.findIndex((m) => m.from >= cursor);
  const current = matches.length === 0 ? -1 : after === -1 ? 0 : after;
  view.dispatch(view.state.tr.setMeta(findKey, { query, matchCase, current }));
}

/** Go to the next (1) or previous (-1) match, round the end. */
export function step(view: EditorView, delta: 1 | -1): void {
  const state = findState(view.state);
  if (!state || state.matches.length === 0) return;
  const count = state.matches.length;
  const current = (((state.current + delta) % count) + count) % count;
  view.dispatch(select(view.state.tr.setMeta(findKey, { current }), state.matches[current]));
}

/** Replace the current match, then move to the next one. */
export function replaceCurrent(view: EditorView, replacement: string): void {
  const state = findState(view.state);
  const match = state?.matches[state.current];
  if (!state || !match) return;
  const tr = view.state.tr;
  if (replacement === '') tr.delete(match.from, match.to);
  else tr.insertText(replacement, match.from, match.to);
  tr.setMeta(findKey, { current: state.current });
  view.dispatch(tr);
  const after = findState(view.state);
  const next = after?.matches[after.current];
  if (next) view.dispatch(select(view.state.tr, next));
}

/** Replace every match; returns how many were replaced. */
export function replaceAll(view: EditorView, replacement: string): number {
  const state = findState(view.state);
  if (!state || state.matches.length === 0) return 0;
  const tr = view.state.tr;
  // From the end, so earlier positions stay right.
  for (const match of [...state.matches].reverse()) {
    if (replacement === '') tr.delete(match.from, match.to);
    else tr.insertText(replacement, match.from, match.to);
  }
  view.dispatch(tr.setMeta(findKey, { current: 0 }));
  return state.matches.length;
}

/** Stop highlighting matches. */
export function clearSearch(view: EditorView): void {
  view.dispatch(view.state.tr.setMeta(findKey, { query: '', current: -1 }));
}
