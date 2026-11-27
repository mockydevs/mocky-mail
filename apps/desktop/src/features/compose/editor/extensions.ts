import { Extension, Node, type AnyExtension, type CommandProps } from '@tiptap/core';
import Highlight from '@tiptap/extension-highlight';
import { Table, TableCell, TableHeader, TableRow } from '@tiptap/extension-table';
import Image from '@tiptap/extension-image';
import TextAlign from '@tiptap/extension-text-align';
import { Color, FontFamily, FontSize, TextStyle } from '@tiptap/extension-text-style';
import { Placeholder } from '@tiptap/extensions';
import type { EditorState } from '@tiptap/pm/state';
import StarterKit from '@tiptap/starter-kit';

import { QUOTE_ATTR, SIGNATURE_ATTR } from '../richText';
import { FindReplace } from './findReplace';

/** Lists nest this deep, no deeper. */
export const MAX_LIST_DEPTH = 8;
/** Paragraph indent steps, like the lists. */
export const MAX_INDENT = 8;

/** Only web and email links: no javascript:, data: or file: from a paste. */
export function allowedLink(url: string): boolean {
  return /^(https?:\/\/[^\s]+|mailto:[^\s]+)$/i.test(url.trim());
}

/** How many list items the cursor is inside. */
export function listDepth(state: EditorState): number {
  const { $from } = state.selection;
  let depth = 0;
  for (let d = $from.depth; d > 0; d--) {
    if ($from.node(d).type.name === 'listItem') depth++;
  }
  return depth;
}

/**
 * The original under a reply or forward (`<div data-mocky-quote>`: the
 * "On … wrote:" line and the quote). The backend puts the signature above it.
 */
const MockyQuote = Node.create({
  name: 'mockyQuote',
  group: 'block',
  content: 'block+',
  defining: true,
  parseHTML() {
    // Ahead of the ordinary blockquote, which would drop the marker.
    return [
      { tag: `div[${QUOTE_ATTR}]`, priority: 100 },
      { tag: `blockquote[${QUOTE_ATTR}]`, priority: 100 },
    ];
  },
  renderHTML() {
    return ['div', { [QUOTE_ATTR]: '' }, 0];
  },
});

/** The signature block, so it can be swapped when From or the picker changes. */
const MockySignature = Node.create({
  name: 'mockySignature',
  group: 'block',
  content: 'block+',
  addAttributes() {
    return {
      // Which of the account's signatures this is; the backend sends it in full here.
      signatureId: {
        default: '',
        parseHTML: (element) => element.getAttribute(SIGNATURE_ATTR) ?? '',
        renderHTML: (attributes) => ({ [SIGNATURE_ATTR]: String(attributes['signatureId'] ?? '') }),
      },
    };
  },
  parseHTML() {
    return [{ tag: `div[${SIGNATURE_ATTR}]`, priority: 100 }];
  },
  renderHTML({ HTMLAttributes }) {
    return ['div', HTMLAttributes, 0];
  },
});

declare module '@tiptap/core' {
  interface Commands<ReturnType> {
    mockyIndent: {
      /** Indent: one level deeper in a list, or the paragraph moves right. */
      indent: () => ReturnType;
      /** The other way. */
      outdent: () => ReturnType;
    };
  }
}

const INDENTED = ['paragraph', 'heading'];

/**
 * Indent for paragraphs (a left margin, which mail programs show), and Tab /
 * Shift+Tab in lists, at most MAX_LIST_DEPTH levels deep.
 */
const Indent = Extension.create({
  name: 'mockyIndent',
  // Before the list item's own Tab, so the depth limit applies.
  priority: 200,
  addGlobalAttributes() {
    return [
      {
        types: INDENTED,
        attributes: {
          indent: {
            default: 0,
            parseHTML: (element) => {
              const margin = /^([\d.]+)em$/.exec(element.style.marginLeft);
              return margin ? Math.min(MAX_INDENT, Math.round(Number(margin[1]) / 2)) : 0;
            },
            renderHTML: (attributes) => {
              const level = Number(attributes['indent'] ?? 0);
              return level > 0 ? { style: `margin-left: ${String(level * 2)}em` } : {};
            },
          },
        },
      },
    ];
  },
  addCommands() {
    const step =
      (delta: number) =>
      () =>
      ({ state, tr, commands }: CommandProps) => {
        if (listDepth(state) > 0) {
          if (delta < 0) return commands.liftListItem('listItem');
          return listDepth(state) < MAX_LIST_DEPTH && commands.sinkListItem('listItem');
        }
        const { from, to } = state.selection;
        let changed = false;
        state.doc.nodesBetween(from, to, (node, pos) => {
          if (!INDENTED.includes(node.type.name)) return true;
          const level = Number(node.attrs['indent'] ?? 0);
          const next = Math.max(0, Math.min(MAX_INDENT, level + delta));
          if (next !== level) {
            tr.setNodeMarkup(pos, undefined, { ...node.attrs, indent: next });
            changed = true;
          }
          return false;
        });
        return changed;
      };
    return { indent: step(1), outdent: step(-1) };
  },
  addKeyboardShortcuts() {
    return {
      Tab: () => {
        if (listDepth(this.editor.state) === 0) return false;
        // Deep enough: keep the Tab (it would otherwise leave the editor).
        if (listDepth(this.editor.state) >= MAX_LIST_DEPTH) return true;
        return this.editor.commands.sinkListItem('listItem') || true;
      },
      'Shift-Tab': () => {
        if (listDepth(this.editor.state) === 0) return false;
        return this.editor.commands.liftListItem('listItem') || true;
      },
      'Mod-]': () => this.editor.commands.indent(),
      'Mod-[': () => this.editor.commands.outdent(),
      'Mod-\\': () => this.editor.chain().unsetAllMarks().clearNodes().run(),
    };
  },
});

/**
 * Pictures that travel inside the message: pasted ones and those in a saved
 * draft or signature (`data:` URLs; the backend sends them as parts of the
 * message). Pictures from the internet are not loaded or kept.
 */
const InlinePicture = Image.extend({
  parseHTML() {
    return [
      {
        tag: 'img[src]',
        getAttrs: (element) => (/^data:image\/(png|jpe?g|gif|webp);base64,/i.test(element.getAttribute('src') ?? '') ? null : false),
      },
    ];
  },
}).configure({ inline: true, allowBase64: true });

export function editorExtensions(placeholder: string): AnyExtension[] {
  return [
    StarterKit.configure({
      codeBlock: false,
      // An empty paragraph added after a quote would count as an edit the user made.
      trailingNode: false,
      code: false,
      link: {
        openOnClick: false,
        autolink: true,
        protocols: ['mailto'],
        defaultProtocol: 'https',
        isAllowedUri: (url) => allowedLink(url),
      },
    }),
    TextStyle,
    Color,
    FontFamily,
    FontSize,
    Highlight.configure({ multicolor: true }),
    TextAlign.configure({ types: ['heading', 'paragraph'] }),
    Table.configure({ resizable: false }),
    TableRow,
    TableHeader,
    TableCell,
    Placeholder.configure({ placeholder }),
    InlinePicture,
    MockyQuote,
    MockySignature,
    Indent,
    FindReplace,
  ];
}
