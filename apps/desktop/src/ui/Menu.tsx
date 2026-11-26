import { useEffect, useId, useRef, useState, type KeyboardEvent, type MouseEvent, type ReactNode } from 'react';

import { Icon, type IconName } from './Icon';

const ITEMS = '[role="menuitem"]:not([disabled]), [role="menuitemradio"]:not([disabled]), [role="menuitemcheckbox"]:not([disabled])';

interface MenuButtonProps {
  /** Accessible name of the button that opens the menu. */
  label: string;
  /** Name of the menu itself; defaults to `label`. */
  menuLabel?: string;
  icon?: IconName;
  /** Visible text beside the icon; icon-only when missing. */
  text?: ReactNode;
  title?: string;
  className?: string;
  disabled?: boolean;
  /** Which edge of the button the menu lines up with. */
  align?: 'left' | 'right';
  /** Pressed look: what the menu chooses is in effect. */
  active?: boolean;
  children: ReactNode;
}

/**
 * A button with a drop-down menu (WAI-ARIA menu button). Opening focuses the
 * first item; arrows, Home and End move between items; Esc or Tab closes and
 * focus goes back to the button. Choosing an item closes the menu unless the
 * item says to keep it open (`keepOpen`). A click outside closes it too.
 */
export function MenuButton({ label, menuLabel, icon, text, title, className, disabled, align = 'right', active, children }: MenuButtonProps) {
  const [open, setOpen] = useState(false);
  const id = useId();
  const buttonRef = useRef<HTMLButtonElement>(null);
  const wrapRef = useRef<HTMLDivElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    menuRef.current?.querySelector<HTMLElement>(ITEMS)?.focus();
    const outside = (event: Event) => {
      if (event.target instanceof Node && wrapRef.current?.contains(event.target)) return;
      setOpen(false);
    };
    document.addEventListener('mousedown', outside);
    document.addEventListener('touchstart', outside);
    return () => {
      document.removeEventListener('mousedown', outside);
      document.removeEventListener('touchstart', outside);
    };
  }, [open]);

  const close = (refocus: boolean) => {
    setOpen(false);
    if (refocus) buttonRef.current?.focus();
  };

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const items = [...(menuRef.current?.querySelectorAll<HTMLElement>(ITEMS) ?? [])];
    const at = items.findIndex((el) => el === document.activeElement);
    const go = (index: number) => {
      event.preventDefault();
      items[(index + items.length) % items.length]?.focus();
    };
    switch (event.key) {
      case 'ArrowDown':
        go(at + 1);
        break;
      case 'ArrowUp':
        go(at - 1);
        break;
      case 'Home':
        go(0);
        break;
      case 'End':
        go(items.length - 1);
        break;
      case 'Escape':
        // The menu takes Esc; it must not also close the screen underneath.
        event.preventDefault();
        event.stopPropagation();
        close(true);
        break;
      case 'Tab':
        close(false);
        break;
    }
    // Letters choose items, not mail shortcuts.
    if (event.key.length === 1) event.stopPropagation();
  };

  const onClick = (event: MouseEvent<HTMLDivElement>) => {
    const item = event.target instanceof Element ? event.target.closest(ITEMS) : null;
    if (item && !item.hasAttribute('data-keep-open')) close(true);
  };

  return (
    <div className={`menu-wrap menu-wrap--${align}`} ref={wrapRef}>
      <button
        ref={buttonRef}
        type="button"
        className={`${className ?? 'icon-btn'}${active ? ' is-active' : ''}`}
        aria-label={label}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-controls={open ? `${id}-menu` : undefined}
        title={title}
        disabled={disabled}
        onClick={() => {
          setOpen((on) => !on);
        }}
      >
        {icon && <Icon name={icon} />}
        {text}
        {text !== undefined && <Icon name="chevron" size={14} />}
      </button>
      {open && (
        <div
          id={`${id}-menu`}
          ref={menuRef}
          role="menu"
          tabIndex={-1}
          aria-label={menuLabel ?? label}
          className="menu"
          onKeyDown={onKeyDown}
          onClick={onClick}
        >
          {children}
        </div>
      )}
    </div>
  );
}

interface MenuItemProps {
  children: ReactNode;
  onSelect: () => void;
  icon?: IconName;
  /** A radio or check item: whether it is on ('mixed': on for some of the messages). */
  checked?: boolean | 'mixed';
  kind?: 'item' | 'radio' | 'check';
  disabled?: boolean;
  danger?: boolean;
  /** Choosing it leaves the menu open (ticking several categories). */
  keepOpen?: boolean;
  /** Overrides the visible text as the accessible name. */
  label?: string;
}

export function MenuItem({ children, onSelect, icon, checked, kind = 'item', disabled, danger, keepOpen, label }: MenuItemProps) {
  const role = kind === 'radio' ? 'menuitemradio' : kind === 'check' ? 'menuitemcheckbox' : 'menuitem';
  return (
    <button
      type="button"
      role={role}
      tabIndex={-1}
      className={`menu__item${danger ? ' menu__item--danger' : ''}`}
      aria-checked={kind === 'item' ? undefined : (checked ?? false)}
      aria-label={label}
      disabled={disabled}
      data-keep-open={keepOpen ? '' : undefined}
      onClick={onSelect}
    >
      {kind !== 'item' ? (
        <span className="menu__tick" aria-hidden="true">
          {checked === true && <Icon name="check" size={16} />}
          {checked === 'mixed' && <span className="menu__mixed" />}
        </span>
      ) : (
        icon && <Icon name={icon} />
      )}
      {children}
    </button>
  );
}

/** Items that belong together, under a small heading. */
export function MenuGroup({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div role="group" aria-label={label} className="menu__group">
      <p className="menu__heading" aria-hidden="true">
        {label}
      </p>
      {children}
    </div>
  );
}

export function MenuSeparator() {
  return <div role="separator" className="menu__sep" />;
}

/**
 * A menu at a point on screen: right-click on a computer, or a long press on
 * a phone (which the browser reports as a context menu). Same keys as a
 * menu button; Esc, Tab, a click outside or choosing an item closes it.
 */
export function ContextMenu({ x, y, label, onClose, children }: { x: number; y: number; label: string; onClose: () => void; children: ReactNode }) {
  const ref = useRef<HTMLDivElement>(null);
  const closeRef = useRef(onClose);
  useEffect(() => {
    closeRef.current = onClose;
  });
  useEffect(() => {
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    ref.current?.querySelector<HTMLElement>(ITEMS)?.focus();
    const outside = (event: Event) => {
      if (event.target instanceof Node && ref.current?.contains(event.target)) return;
      closeRef.current();
    };
    document.addEventListener('mousedown', outside);
    document.addEventListener('touchstart', outside);
    return () => {
      document.removeEventListener('mousedown', outside);
      document.removeEventListener('touchstart', outside);
      previous?.focus();
    };
  }, []);

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const items = [...(ref.current?.querySelectorAll<HTMLElement>(ITEMS) ?? [])];
    const at = items.findIndex((el) => el === document.activeElement);
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault();
      const next = event.key === 'ArrowDown' ? at + 1 : at - 1;
      items[(next + items.length) % items.length]?.focus();
    } else if (event.key === 'Escape' || event.key === 'Tab') {
      event.preventDefault();
      event.stopPropagation();
      onClose();
    }
    if (event.key.length === 1) event.stopPropagation();
  };

  // Keep it on screen near the edges.
  const left = Math.max(8, Math.min(x, window.innerWidth - 240));
  const top = Math.max(8, Math.min(y, window.innerHeight - 160));
  return (
    <div
      ref={ref}
      role="menu"
      tabIndex={-1}
      aria-label={label}
      className="menu menu--context"
      style={{ left, top }}
      onKeyDown={onKeyDown}
      onClick={(event) => {
        if (event.target instanceof Element && event.target.closest(ITEMS)) onClose();
      }}
    >
      {children}
    </div>
  );
}
