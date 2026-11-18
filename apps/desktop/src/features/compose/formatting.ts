/** The text and selection after a formatting button. */
export interface Edit {
  value: string;
  start: number;
  end: number;
}

/**
 * Wrap the selection in marks (`**`, `*`, `` ` ``). With nothing selected,
 * insert a placeholder between the marks and select it so typing replaces
 * it. Pressing again on already-wrapped text removes the marks.
 */
export function wrap(value: string, start: number, end: number, mark: string, placeholder: string): Edit {
  const selected = value.slice(start, end);
  const before = value.slice(0, start);
  const after = value.slice(end);
  if (before.endsWith(mark) && after.startsWith(mark) && selected !== '') {
    return {
      value: before.slice(0, -mark.length) + selected + after.slice(mark.length),
      start: start - mark.length,
      end: end - mark.length,
    };
  }
  const text = selected === '' ? placeholder : selected;
  return {
    value: `${before}${mark}${text}${mark}${after}`,
    start: start + mark.length,
    end: start + mark.length + text.length,
  };
}

/** Turn the selection into a link: `[text](https://)`, with the address selected. */
export function link(value: string, start: number, end: number): Edit {
  const selected = value.slice(start, end);
  const looksLikeUrl = /^https?:\/\/\S+$/i.test(selected);
  const text = looksLikeUrl || selected === '' ? 'link text' : selected;
  const url = looksLikeUrl ? selected : 'https://';
  const inserted = `[${text}](${url})`;
  const urlStart = start + text.length + 3;
  return {
    value: value.slice(0, start) + inserted + value.slice(end),
    start: looksLikeUrl || selected === '' ? start + 1 : urlStart,
    end: looksLikeUrl || selected === '' ? start + 1 + text.length : urlStart + url.length,
  };
}

/**
 * Start every selected line with a prefix (`- `, `> `, or `1. `, `2. `…
 * when `numbered`). Lines that already have it lose it instead.
 */
export function prefixLines(value: string, start: number, end: number, prefix: string, numbered = false): Edit {
  const lineStart = value.lastIndexOf('\n', start - 1) + 1;
  const nextBreak = value.indexOf('\n', Math.max(end - 1, start));
  const lineEnd = nextBreak === -1 ? value.length : nextBreak;
  const lines = value.slice(lineStart, lineEnd).split('\n');
  const marker = (i: number) => (numbered ? `${String(i + 1)}. ` : prefix);
  const has = (line: string, i: number) => (numbered ? /^\d+\. /.test(line) : line.startsWith(marker(i)));
  const remove = lines.every((line, i) => has(line, i));
  const changed = lines.map((line, i) => (remove ? line.replace(numbered ? /^\d+\. / : prefix, '') : marker(i) + line));
  const block = changed.join('\n');
  return {
    value: value.slice(0, lineStart) + block + value.slice(lineEnd),
    start: lineStart,
    end: lineStart + block.length,
  };
}
