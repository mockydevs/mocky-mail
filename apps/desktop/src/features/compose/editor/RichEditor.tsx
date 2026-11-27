import type { Mark } from '@tiptap/pm/model';
import { EditorContent, useEditor, type Editor } from '@tiptap/react';
import {
  useEffect,
  useId,
  useImperativeHandle,
  useRef,
  useState,
  type KeyboardEvent,
  type MouseEvent,
  type ReactNode,
} from 'react';

import { Icon } from '../../../ui/Icon';
import { MenuButton, MenuItem, MenuSeparator } from '../../../ui/Menu';
import { Modal } from '../../../ui/Modal';
import { htmlToPlainText } from '../richText';
import { allowedLink, editorExtensions, listDepth, MAX_LIST_DEPTH } from './extensions';
import { clearSearch, findState, replaceAll, replaceCurrent, setSearch, step } from './findReplace';
import { marksAt, paintSelection } from './formatPainter';
import type { RichEditorProps } from './loader';

/** Fonts every mail program can show. Empty: the reader's default. */
export const FONTS: readonly [string, string][] = [
  ['', 'Default font'],
  ['Arial, Helvetica, sans-serif', 'Arial'],
  ['Verdana, Geneva, sans-serif', 'Verdana'],
  ['Tahoma, Geneva, sans-serif', 'Tahoma'],
  ['"Trebuchet MS", Helvetica, sans-serif', 'Trebuchet MS'],
  ['Georgia, serif', 'Georgia'],
  ['"Times New Roman", Times, serif', 'Times New Roman'],
  ['"Courier New", Courier, monospace', 'Courier New'],
];

export const SIZES: readonly [string, string][] = [
  ['12px', 'Small'],
  ['', 'Normal'],
  ['18px', 'Large'],
  ['24px', 'Huge'],
];

const TEXT_COLOURS: readonly [string, string][] = [
  ['#000000', 'Black'],
  ['#555555', 'Grey'],
  ['#c0392b', 'Red'],
  ['#d35400', 'Orange'],
  ['#1e8449', 'Green'],
  ['#1f5fbf', 'Blue'],
  ['#7d3c98', 'Purple'],
];

const HIGHLIGHTS: readonly [string, string][] = [
  ['#fff59d', 'Yellow'],
  ['#c8e6c9', 'Green'],
  ['#bbdefb', 'Blue'],
  ['#f8bbd0', 'Pink'],
  ['#ffe0b2', 'Orange'],
  ['#e0e0e0', 'Grey'],
];

const ALIGNMENTS: readonly [string, string][] = [
  ['left', 'Align left'],
  ['center', 'Centre'],
  ['right', 'Align right'],
  ['justify', 'Justify'],
];

/** The first family of a CSS font list, for matching what the editor reports. */
function firstFamily(fonts: string): string {
  return (fonts.split(',')[0] ?? '').replace(/["']/g, '').trim().toLowerCase();
}

function textOf(editor: Editor): string {
  return htmlToPlainText(editor.getHTML());
}

interface ButtonProps {
  label: string;
  shortcut?: string;
  active?: boolean;
  disabled?: boolean;
  onClick: (event: MouseEvent<HTMLButtonElement>) => void;
  onDoubleClick?: () => void;
  children: ReactNode;
}

function ToolButton({ label, shortcut, active, disabled, onClick, onDoubleClick, children }: ButtonProps) {
  return (
    <button
      type="button"
      className="rte__btn"
      aria-label={label}
      aria-pressed={active}
      title={shortcut ? `${label} (${shortcut})` : label}
      disabled={disabled}
      // Keep the selection in the editor while clicking.
      onMouseDown={(e) => {
        e.preventDefault();
      }}
      onClick={onClick}
      onDoubleClick={onDoubleClick}
    >
      {children}
    </button>
  );
}

function Swatch({ colour }: { colour: string | null }) {
  return <span className="rte__swatch" style={colour ? { background: colour } : undefined} aria-hidden="true" />;
}

interface LinkDraft {
  text: string;
  url: string;
  /** Editing a link that is already there. */
  existing: boolean;
}

function LinkDialog({ initial, onCancel, onSave, onRemove }: {
  initial: LinkDraft;
  onCancel: () => void;
  onSave: (text: string, url: string) => void;
  onRemove: () => void;
}) {
  const id = useId();
  const [text, setText] = useState(initial.text);
  const [url, setUrl] = useState(initial.url);
  const [error, setError] = useState<string | null>(null);
  const save = () => {
    let address = url.trim();
    // "example.com" means the web page; "someone@example.com" an email address.
    if (address !== '' && !/^[a-z][a-z0-9+.-]*:/i.test(address)) {
      address = /^[^\s@/]+@[^\s@/]+\.[^\s@/]+$/.test(address) ? `mailto:${address}` : `https://${address}`;
    }
    if (!allowedLink(address)) {
      setError('Use a web address (http:// or https://) or an email address (mailto:).');
      return;
    }
    onSave(text.trim() === '' ? address.replace(/^mailto:/i, '') : text, address);
  };
  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'Enter') {
      event.preventDefault();
      event.stopPropagation();
      save();
    }
  };
  return (
    <Modal title={initial.existing ? 'Edit link' : 'Insert link'} size="small" onClose={onCancel}>
      <div className="form-field">
        <label htmlFor={`${id}-text`} className="form-field__label">
          Text to show
        </label>
        <input
          id={`${id}-text`}
          className="field"
          value={text}
          onChange={(e) => {
            setText(e.target.value);
          }}
          onKeyDown={onKeyDown}
        />
      </div>
      <div className="form-field">
        <label htmlFor={`${id}-url`} className="form-field__label">
          Address
        </label>
        <input
          id={`${id}-url`}
          className="field"
          type="url"
          inputMode="url"
          placeholder="https://"
          value={url}
          aria-invalid={error !== null ? true : undefined}
          aria-describedby={error !== null ? `${id}-error` : undefined}
          onChange={(e) => {
            setUrl(e.target.value);
            setError(null);
          }}
          onKeyDown={onKeyDown}
        />
        {error && (
          <p id={`${id}-error`} className="form-error" role="alert">
            {error}
          </p>
        )}
      </div>
      <div className="modal__actions">
        {initial.existing && (
          <button type="button" className="btn btn--secondary" onClick={onRemove}>
            Remove link
          </button>
        )}
        <button type="button" className="btn btn--secondary" onClick={onCancel}>
          Cancel
        </button>
        <button type="button" className="btn btn--primary" onClick={save}>
          {initial.existing ? 'Save link' : 'Insert link'}
        </button>
      </div>
    </Modal>
  );
}

function FindBar({ editor, replacing, onReplacing, onClose }: {
  editor: Editor;
  replacing: boolean;
  onReplacing: (on: boolean) => void;
  onClose: () => void;
}) {
  const id = useId();
  const [query, setQuery] = useState(() => {
    const { from, to, empty } = editor.state.selection;
    const selected = empty ? '' : editor.state.doc.textBetween(from, to, ' ');
    return selected.includes('\n') ? '' : selected;
  });
  const [replacement, setReplacement] = useState('');
  const [matchCase, setMatchCase] = useState(false);
  const [done, setDone] = useState<string | null>(null);
  const findRef = useRef<HTMLInputElement>(null);
  const state = findState(editor.state);
  const count = state?.matches.length ?? 0;
  const current = state?.current ?? -1;

  useEffect(() => {
    findRef.current?.focus();
    findRef.current?.select();
  }, [replacing]);

  useEffect(() => {
    setSearch(editor.view, query, matchCase);
  }, [editor, query, matchCase]);

  useEffect(
    () => () => {
      if (!editor.isDestroyed) clearSearch(editor.view);
    },
    [editor],
  );

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      onClose();
    } else if (event.key === 'Enter' && event.target === findRef.current) {
      event.preventDefault();
      event.stopPropagation();
      step(editor.view, event.shiftKey ? -1 : 1);
    } else if (event.key === 'Enter') {
      event.preventDefault();
      event.stopPropagation();
    }
  };

  const status = query === '' ? '' : count === 0 ? 'No matches' : `${String(current + 1)} of ${String(count)}`;

  return (
    // Keys inside the bar (Enter, Esc) belong to it, not to compose.
    // eslint-disable-next-line jsx-a11y/no-noninteractive-element-interactions
    <div className="rte__find" role="search" aria-label="Find and replace" onKeyDown={onKeyDown}>
      <div className="rte__find-row">
        <label htmlFor={`${id}-find`} className="visually-hidden">
          Find
        </label>
        <input
          ref={findRef}
          id={`${id}-find`}
          className="field rte__find-input"
          placeholder="Find"
          value={query}
          onChange={(e) => {
            setQuery(e.target.value);
            setDone(null);
          }}
        />
        <span className="rte__find-count muted small" role="status">
          {done ?? status}
        </span>
        <button type="button" className="icon-btn" aria-label="Previous match" disabled={count === 0} onClick={() => { step(editor.view, -1); }}>
          <Icon name="left" />
        </button>
        <button type="button" className="icon-btn" aria-label="Next match" disabled={count === 0} onClick={() => { step(editor.view, 1); }}>
          <Icon name="right" />
        </button>
        <label className="rte__find-case">
          <input
            type="checkbox"
            checked={matchCase}
            onChange={(e) => {
              setMatchCase(e.target.checked);
            }}
          />
          Match case
        </label>
        {!replacing && (
          <button type="button" className="btn btn--link" onClick={() => { onReplacing(true); }}>
            Replace…
          </button>
        )}
        <button type="button" className="icon-btn" aria-label="Close find" onClick={onClose}>
          <Icon name="close" />
        </button>
      </div>
      {replacing && (
        <div className="rte__find-row">
          <label htmlFor={`${id}-replace`} className="visually-hidden">
            Replace with
          </label>
          <input
            id={`${id}-replace`}
            className="field rte__find-input"
            placeholder="Replace with"
            value={replacement}
            onChange={(e) => {
              setReplacement(e.target.value);
            }}
          />
          <button type="button" className="btn btn--secondary btn--small" disabled={count === 0} onClick={() => { replaceCurrent(editor.view, replacement); }}>
            Replace
          </button>
          <button
            type="button"
            className="btn btn--secondary btn--small"
            disabled={count === 0}
            onClick={() => {
              const n = replaceAll(editor.view, replacement);
              setDone(n === 1 ? 'Replaced 1' : `Replaced ${String(n)}`);
              // This button is off now that nothing matches; stay in the bar.
              findRef.current?.focus();
            }}
          >
            Replace all
          </button>
        </div>
      )}
    </div>
  );
}

/** Painter: the marks it copied, and whether it stays on after one use (double-click). */
interface Painter {
  marks: readonly Mark[];
  sticky: boolean;
}

export function RichEditor({ label, value, onChange, onNormalize, focusOnOpen, handleRef, phone }: RichEditorProps) {
  /** The HTML the editor holds, as last reported; a different `value` comes from outside. */
  const last = useRef(value);
  /**
   * What the editor reported since compose last caught up. Compose can render
   * one of these after the user has typed on: that is an echo, not new content.
   */
  const reported = useRef(new Set<string>());
  const changeRef = useRef(onChange);
  const normalizeRef = useRef(onNormalize);
  useEffect(() => {
    changeRef.current = onChange;
    normalizeRef.current = onNormalize;
  });
  const [link, setLink] = useState<LinkDraft | null>(null);
  const [find, setFind] = useState<null | 'find' | 'replace'>(null);
  const [painter, setPainter] = useState<Painter | null>(null);
  const painterRef = useRef(painter);
  useEffect(() => {
    painterRef.current = painter;
  });

  // The same objects every render: new ones would reconfigure the editor
  // while the user types.
  const [setup] = useState(() => ({
    extensions: editorExtensions('Write your message'),
    // Only what it starts with; later values are set below.
    content: value,
    editorProps: {
      attributes: {
        class: 'rte__content compose__body',
        role: 'textbox',
        'aria-multiline': 'true',
        'aria-label': label,
        spellcheck: 'true',
      },
    },
  }));
  const editor = useEditor({
    ...setup,
    shouldRerenderOnTransaction: true,
    onCreate: ({ editor: created }) => {
      const html = created.getHTML();
      const from = last.current;
      last.current = html;
      if (html !== from) normalizeRef.current(from, html, textOf(created));
    },
    onUpdate: ({ editor: changed }) => {
      const html = changed.getHTML();
      last.current = html;
      reported.current.add(html);
      changeRef.current(html, textOf(changed));
    },
  });

  // Replies start at the top, above the quoted original.
  useEffect(() => {
    if (focusOnOpen) editor.commands.focus('start');
    // Once, when the editor is first on screen.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [editor]);

  // A new value from outside (signature swapped): show it.
  useEffect(() => {
    if (value === last.current) reported.current.clear();
    if (value === last.current || reported.current.has(value) || editor.isDestroyed) return;
    reported.current.clear();
    editor.commands.setContent(value, { emitUpdate: false });
    const html = editor.getHTML();
    last.current = html;
    if (html !== value) normalizeRef.current(value, html, textOf(editor));
  }, [editor, value]);

  useImperativeHandle(
    handleRef,
    () => ({
      focus: () => {
        editor.commands.focus();
      },
      insertText: (text: string) => {
        editor.chain().focus().insertContent(text).run();
      },
    }),
    [editor],
  );

  const openLink = () => {
    const chain = editor.chain().focus();
    if (editor.isActive('link')) chain.extendMarkRange('link').run();
    const { from, to } = editor.state.selection;
    const href = (editor.getAttributes('link')['href'] as string | undefined) ?? '';
    setLink({ text: editor.state.doc.textBetween(from, to, ' '), url: href, existing: href !== '' });
  };

  const saveLink = (text: string, url: string) => {
    setLink(null);
    const { from, to } = editor.state.selection;
    const selected = editor.state.doc.textBetween(from, to, ' ');
    if (from !== to && selected === text) {
      editor.chain().focus().extendMarkRange('link').setLink({ href: url }).run();
    } else {
      editor
        .chain()
        .focus()
        .insertContentAt({ from, to }, { type: 'text', text, marks: [{ type: 'link', attrs: { href: url } }] })
        .run();
    }
  };

  const startPainter = (sticky: boolean) => {
    setPainter({ marks: marksAt(editor.state), sticky });
    editor.commands.focus();
  };

  // The painter applies to the next selection the user makes.
  const paint = () => {
    const active = painterRef.current;
    if (!active) return;
    if (paintSelection(editor.view, active.marks) && !active.sticky) setPainter(null);
  };

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const mod = (event.ctrlKey || event.metaKey) && !event.altKey;
    const key = event.key.toLowerCase();
    if (mod && !event.shiftKey && key === 'k') {
      event.preventDefault();
      openLink();
    } else if (mod && !event.shiftKey && key === 'f') {
      event.preventDefault();
      event.stopPropagation();
      setFind((f) => f ?? 'find');
    } else if (mod && !event.shiftKey && key === 'h') {
      event.preventDefault();
      event.stopPropagation();
      setFind('replace');
    } else if (event.key === 'Escape' && painter) {
      event.stopPropagation();
      setPainter(null);
    }
  };

  const textStyle = editor.getAttributes('textStyle') as { fontFamily?: string; fontSize?: string; color?: string };
  const font = FONTS.find(([value]) => value !== '' && firstFamily(value) === firstFamily(textStyle.fontFamily ?? ''))?.[0] ?? '';
  const size = SIZES.find(([value]) => value !== '' && value === textStyle.fontSize)?.[0] ?? '';
  const inTable = editor.isActive('table');
  const can = editor.can();

  const toolbar = (
    <div className="rte__bar">
      <div className="rte__toolbar" role="toolbar" aria-label="Formatting">
        <ToolButton label="Bold" shortcut="Ctrl+B" active={editor.isActive('bold')} onClick={() => editor.chain().focus().toggleBold().run()}>
          <b>B</b>
        </ToolButton>
        <ToolButton label="Italic" shortcut="Ctrl+I" active={editor.isActive('italic')} onClick={() => editor.chain().focus().toggleItalic().run()}>
          <i style={{ fontFamily: 'Georgia, serif' }}>I</i>
        </ToolButton>
        <ToolButton label="Underline" shortcut="Ctrl+U" active={editor.isActive('underline')} onClick={() => editor.chain().focus().toggleUnderline().run()}>
          <u>U</u>
        </ToolButton>
        <ToolButton label="Strikethrough" shortcut="Ctrl+Shift+S" active={editor.isActive('strike')} onClick={() => editor.chain().focus().toggleStrike().run()}>
          <s>S</s>
        </ToolButton>
        <select
          className="rte__select"
          aria-label="Font"
          value={font}
          onChange={(e) => {
            const next = e.target.value;
            if (next === '') editor.chain().focus().unsetFontFamily().run();
            else editor.chain().focus().setFontFamily(next).run();
          }}
        >
          {FONTS.map(([value, name]) => (
            <option key={name} value={value}>
              {name}
            </option>
          ))}
        </select>
        <select
          className="rte__select"
          aria-label="Size"
          value={size}
          onChange={(e) => {
            const next = e.target.value;
            if (next === '') editor.chain().focus().unsetFontSize().run();
            else editor.chain().focus().setFontSize(next).run();
          }}
        >
          {SIZES.map(([value, name]) => (
            <option key={name} value={value}>
              {name}
            </option>
          ))}
        </select>
        <MenuButton label="Text colour" className="rte__btn" align="left" text={<Swatch colour={textStyle.color ?? null} />}>
          <MenuItem kind="radio" checked={!textStyle.color} onSelect={() => editor.chain().focus().unsetColor().run()}>
            <Swatch colour={null} /> Automatic
          </MenuItem>
          {TEXT_COLOURS.map(([colour, name]) => (
            <MenuItem key={colour} kind="radio" checked={textStyle.color === colour} onSelect={() => editor.chain().focus().setColor(colour).run()}>
              <Swatch colour={colour} /> {name}
            </MenuItem>
          ))}
        </MenuButton>
        <MenuButton label="Highlight" className="rte__btn" align="left" text={<span className="rte__mark" aria-hidden="true">ab</span>}>
          <MenuItem kind="radio" checked={!editor.isActive('highlight')} onSelect={() => editor.chain().focus().unsetHighlight().run()}>
            <Swatch colour={null} /> No highlight
          </MenuItem>
          {HIGHLIGHTS.map(([colour, name]) => (
            <MenuItem
              key={colour}
              kind="radio"
              checked={editor.isActive('highlight', { color: colour })}
              onSelect={() => editor.chain().focus().setHighlight({ color: colour }).run()}
            >
              <Swatch colour={colour} /> {name}
            </MenuItem>
          ))}
        </MenuButton>
        <span className="rte__sep" aria-hidden="true" />
        <ToolButton label="Bulleted list" shortcut="Ctrl+Shift+8" active={editor.isActive('bulletList')} onClick={() => editor.chain().focus().toggleBulletList().run()}>
          •
        </ToolButton>
        <ToolButton label="Numbered list" shortcut="Ctrl+Shift+7" active={editor.isActive('orderedList')} onClick={() => editor.chain().focus().toggleOrderedList().run()}>
          1.
        </ToolButton>
        <ToolButton label="Decrease indent" shortcut="Shift+Tab in a list, Ctrl+[" disabled={!can.outdent()} onClick={() => editor.chain().focus().outdent().run()}>
          ⇤
        </ToolButton>
        <ToolButton
          label="Increase indent"
          shortcut="Tab in a list, Ctrl+]"
          disabled={listDepth(editor.state) >= MAX_LIST_DEPTH || !can.indent()}
          onClick={() => editor.chain().focus().indent().run()}
        >
          ⇥
        </ToolButton>
        <MenuButton label="Alignment" className="rte__btn" align="left" text={<span aria-hidden="true">≡</span>}>
          {ALIGNMENTS.map(([value, name]) => (
            <MenuItem key={value} kind="radio" checked={editor.isActive({ textAlign: value })} onSelect={() => editor.chain().focus().setTextAlign(value).run()}>
              {name}
            </MenuItem>
          ))}
        </MenuButton>
        <span className="rte__sep" aria-hidden="true" />
        <ToolButton label="Link" shortcut="Ctrl+K" active={editor.isActive('link')} onClick={openLink}>
          Link
        </ToolButton>
        <ToolButton label="Quote" shortcut="Ctrl+Shift+B" active={editor.isActive('blockquote')} onClick={() => editor.chain().focus().toggleBlockquote().run()}>
          ❝
        </ToolButton>
        <MenuButton label="Table" className="rte__btn" align="left" text={<span aria-hidden="true">▦</span>} active={inTable}>
          <MenuItem onSelect={() => editor.chain().focus().insertTable({ rows: 3, cols: 3, withHeaderRow: false }).run()} disabled={inTable}>
            Insert table (3 × 3)
          </MenuItem>
          <MenuSeparator />
          <MenuItem onSelect={() => editor.chain().focus().addRowAfter().run()} disabled={!inTable}>
            Add row below
          </MenuItem>
          <MenuItem onSelect={() => editor.chain().focus().addColumnAfter().run()} disabled={!inTable}>
            Add column right
          </MenuItem>
          <MenuItem onSelect={() => editor.chain().focus().deleteRow().run()} disabled={!inTable}>
            Delete row
          </MenuItem>
          <MenuItem onSelect={() => editor.chain().focus().deleteColumn().run()} disabled={!inTable}>
            Delete column
          </MenuItem>
          <MenuItem onSelect={() => editor.chain().focus().deleteTable().run()} disabled={!inTable} danger>
            Delete table
          </MenuItem>
        </MenuButton>
        <ToolButton label="Horizontal line" onClick={() => editor.chain().focus().setHorizontalRule().run()}>
          ―
        </ToolButton>
        <span className="rte__sep" aria-hidden="true" />
        <ToolButton
          label={painter?.sticky ? 'Format painter (on until Esc)' : 'Format painter'}
          shortcut="click: once, double-click: until Esc"
          active={painter !== null}
          onClick={(event) => {
            // The second click of a double-click is handled by onDoubleClick.
            if (event.detail > 1) return;
            if (painter) setPainter(null);
            else startPainter(false);
          }}
          onDoubleClick={() => {
            startPainter(true);
          }}
        >
          <Icon name="pen" size={16} />
        </ToolButton>
        <ToolButton label="Clear formatting" shortcut="Ctrl+\" onClick={() => editor.chain().focus().unsetAllMarks().clearNodes().run()}>
          <span aria-hidden="true">T̸</span>
        </ToolButton>
        <ToolButton label="Find and replace" shortcut="Ctrl+F, Ctrl+H" active={find !== null} onClick={() => { setFind((f) => (f ? null : 'find')); }}>
          <Icon name="search" size={16} />
        </ToolButton>
      </div>
    </div>
  );

  return (
    // Editor keys (Ctrl+K, Ctrl+F, Ctrl+H, Esc for the painter) are read as they bubble up.
    // eslint-disable-next-line jsx-a11y/no-static-element-interactions
    <div
      className={`rte${phone ? ' rte--phone' : ''}${painter ? ' rte--painting' : ''}`}
      onKeyDown={onKeyDown}
      onMouseUp={paint}
      onKeyUp={(event) => {
        if (event.shiftKey) paint();
      }}
    >
      {toolbar}
      {find && (
        <FindBar
          editor={editor}
          replacing={find === 'replace'}
          onReplacing={(on) => {
            setFind(on ? 'replace' : 'find');
          }}
          onClose={() => {
            setFind(null);
            editor.commands.focus();
          }}
        />
      )}
      <EditorContent editor={editor} className="rte__area" />
      {link && (
        <LinkDialog
          initial={link}
          onCancel={() => {
            setLink(null);
            editor.commands.focus();
          }}
          onSave={saveLink}
          onRemove={() => {
            setLink(null);
            editor.chain().focus().extendMarkRange('link').unsetLink().run();
          }}
        />
      )}
    </div>
  );
}
