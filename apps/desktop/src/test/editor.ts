import type { Editor } from '@tiptap/core';

import { htmlToPlainText } from '../features/compose/richText';

/** The rich-text editor behind compose's Message box (TipTap keeps itself on its element). */
export function editorOf(box: HTMLElement): Editor {
  const editor = (box as HTMLElement & { editor?: Editor }).editor;
  if (!editor) throw new Error('not a rich-text editor');
  return editor;
}

/** The message as HTML, as compose saves and sends it. */
export const messageHtml = (box: HTMLElement): string => editorOf(box).getHTML();

/** The plain-text version compose keeps beside the HTML. */
export const messageText = (box: HTMLElement): string => htmlToPlainText(messageHtml(box));

/** Select the first place `text` is written in the message (within one run of text). */
export function selectText(box: HTMLElement, text: string): void {
  const editor = editorOf(box);
  const found: { from: number; to: number }[] = [];
  editor.state.doc.descendants((node, pos) => {
    if (found.length > 0) return false;
    const at = node.isText ? (node.text ?? '').indexOf(text) : -1;
    if (at !== -1) found.push({ from: pos + at, to: pos + at + text.length });
    return true;
  });
  const [range] = found;
  if (!range) throw new Error(`"${text}" is not in the message`);
  editor.commands.setTextSelection(range);
}
