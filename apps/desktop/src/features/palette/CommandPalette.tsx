import { useEffect, useId, useRef, useState, type KeyboardEvent } from 'react';

import { Icon } from '../../ui/Icon';

export interface PaletteCommand {
  id: string;
  label: string;
  /** Heading it is listed under ("Go to", "Settings"…), also searched. */
  group: string;
  /** Shown after the label in grey, also searched (an account name, a key). */
  detail?: string;
  /** Other words it is found by. */
  keywords?: string;
  run: () => void;
}

/** How well a command matches: lower is better, null is no match. Every word typed must appear. */
export function paletteRank(command: PaletteCommand, query: string): number | null {
  const words = query.toLocaleLowerCase().split(/\s+/).filter(Boolean);
  if (words.length === 0) return 0;
  const label = command.label.toLocaleLowerCase();
  const hay = [label, command.group, command.detail ?? '', command.keywords ?? ''].join(' ').toLocaleLowerCase();
  if (!words.every((w) => hay.includes(w))) return null;
  const first = words[0] ?? '';
  if (label.startsWith(first)) return 0;
  if (label.split(/[\s›(]+/).some((part) => part.startsWith(first))) return 1;
  return label.includes(first) ? 2 : 3;
}

export function filterCommands(commands: readonly PaletteCommand[], query: string): PaletteCommand[] {
  return commands
    .map((command, index) => ({ command, index, rank: paletteRank(command, query) }))
    .filter((x): x is { command: PaletteCommand; index: number; rank: number } => x.rank !== null)
    .sort((a, b) => a.rank - b.rank || a.index - b.index)
    .map((x) => x.command);
}

const SHOWN = 50;

/**
 * Ctrl+K (Cmd+K on a Mac): type to find any action in the app, arrows to move,
 * Enter to run it, Esc to close. Running a command closes the palette first,
 * so the command can move focus where it wants.
 */
export function CommandPalette({ commands, onClose }: { commands: readonly PaletteCommand[]; onClose: () => void }) {
  const id = useId();
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const [query, setQuery] = useState('');
  const [active, setActive] = useState(0);
  // Focus goes back where it was, unless a command took it somewhere.
  const ran = useRef(false);

  useEffect(() => {
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    inputRef.current?.focus();
    return () => {
      if (!ran.current) previous?.focus();
    };
  }, []);

  const found = filterCommands(commands, query).slice(0, SHOWN);
  const current = Math.min(active, Math.max(0, found.length - 1));
  const activeCommand = found[current];

  useEffect(() => {
    const el = listRef.current?.querySelector<HTMLElement>('[aria-selected="true"]');
    // scrollIntoView is missing in some test DOMs.
    // eslint-disable-next-line @typescript-eslint/no-unnecessary-condition
    el?.scrollIntoView?.({ block: 'nearest' });
  }, [current, query]);

  const choose = (command: PaletteCommand | undefined) => {
    if (!command) return;
    ran.current = true;
    onClose();
    command.run();
  };

  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    switch (event.key) {
      case 'ArrowDown':
        event.preventDefault();
        setActive(found.length === 0 ? 0 : (current + 1) % found.length);
        return;
      case 'ArrowUp':
        event.preventDefault();
        setActive(found.length === 0 ? 0 : (current - 1 + found.length) % found.length);
        return;
      case 'Home':
        event.preventDefault();
        setActive(0);
        return;
      case 'End':
        event.preventDefault();
        setActive(Math.max(0, found.length - 1));
        return;
      case 'Enter':
        event.preventDefault();
        choose(activeCommand);
        return;
      case 'Escape':
        event.preventDefault();
        event.stopPropagation();
        onClose();
        return;
      case 'Tab':
        // Nothing else to tab to: stay in the box.
        event.preventDefault();
        return;
    }
  };

  return (
    <div
      className="modal-backdrop palette-backdrop"
      role="presentation"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="modal palette" role="dialog" aria-modal="true" aria-label="Command palette">
        <div className="palette__search">
          <Icon name="search" />
          <input
            ref={inputRef}
            className="palette__input"
            role="combobox"
            aria-label="Find a command"
            aria-expanded="true"
            aria-controls={`${id}-list`}
            aria-autocomplete="list"
            aria-activedescendant={activeCommand ? `${id}-option-${String(current)}` : undefined}
            placeholder="Find a command, folder or setting…"
            autoComplete="off"
            spellCheck={false}
            value={query}
            onChange={(e) => {
              setQuery(e.target.value);
              setActive(0);
            }}
            onKeyDown={onKeyDown}
          />
          <kbd className="palette__esc">Esc</kbd>
        </div>
        <div ref={listRef} id={`${id}-list`} className="palette__list" role="listbox" aria-label="Commands">
          {found.map((command, index) => {
            // The first of each group gets a little space above it.
            const first = index === 0 || found[index - 1]?.group !== command.group;
            return (
              <div
                key={command.id}
                id={`${id}-option-${String(index)}`}
                role="option"
                tabIndex={-1}
                aria-selected={index === current}
                className={`palette__item${index === current ? ' palette__item--active' : ''}${first ? ' palette__item--first' : ''}`}
                onMouseMove={() => {
                  if (index !== current) setActive(index);
                }}
                onClick={() => {
                  choose(command);
                }}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') choose(command);
                }}
              >
                <span className="palette__label">{command.label}</span>
                {command.detail && <span className="palette__detail">{command.detail}</span>}
                <span className="palette__group">{command.group}</span>
              </div>
            );
          })}
        </div>
        {found.length === 0 && (
          <p className="palette__empty" role="status">
            Nothing matches “{query.trim()}”.
          </p>
        )}
      </div>
    </div>
  );
}
