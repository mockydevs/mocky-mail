import { createElement, useEffect, useState, type ComponentType, type ReactNode, type Ref } from 'react';

/** What compose can ask of the editor. */
export interface EditorHandle {
  focus(): void;
  /** Type this at the cursor (a symbol from Insert). */
  insertText(text: string): void;
}

export interface RichEditorProps {
  /** Accessible name of the editing area. */
  label: string;
  /** The message as HTML. A different value from outside (a signature swap) replaces the content. */
  value: string;
  /** The user changed the message: its HTML and plain-text version. */
  onChange: (html: string, text: string) => void;
  /**
   * The editor tidied `from` (loaded HTML) into `to`. Not a change the user
   * made, so compose treats it like the loaded content.
   */
  onNormalize: (from: string, to: string, text: string) => void;
  /** Put the cursor at the top of the message once it is ready. */
  focusOnOpen: boolean;
  handleRef: Ref<EditorHandle>;
  /** Phone layout: one scrolling toolbar row with big buttons. */
  phone: boolean;
}

type Editor = ComponentType<RichEditorProps>;

/**
 * The rich-text editor (TipTap) is a chunk of its own, so the mail list
 * doesn't wait for it. It starts loading as soon as the app is up, and is
 * normally ready before compose opens.
 */
let loaded: Editor | null = null;
let loading: Promise<Editor> | null = null;

export function preloadEditor(): Promise<Editor> {
  loading ??= import('./RichEditor').then(
    (module) => {
      loaded = module.RichEditor;
      return module.RichEditor;
    },
    (error: unknown) => {
      // Try again next time (a chunk that failed to load).
      loading = null;
      throw error;
    },
  );
  return loading;
}

/** The editor component, or null while its chunk loads. */
export function useRichEditor(): Editor | null {
  // A function: useState would call the component itself as an initialiser.
  const [editor, setEditor] = useState<Editor | null>(() => loaded);
  useEffect(() => {
    if (editor) return;
    let active = true;
    preloadEditor().then(
      (next) => {
        if (active) setEditor(() => next);
      },
      () => undefined,
    );
    return () => {
      active = false;
    };
  }, [editor]);
  return editor;
}

/** The editor once its chunk is here; `fallback` until then. */
export function RichEditorSlot({ fallback, ...props }: RichEditorProps & { fallback: ReactNode }) {
  const editor = useRichEditor();
  return editor ? createElement(editor, props) : fallback;
}
