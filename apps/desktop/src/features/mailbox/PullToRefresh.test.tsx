import { act, fireEvent, render, screen } from '@testing-library/react';
import { vi } from 'vitest';

import { MessageList } from './MessageList';
import { PULL_THRESHOLD } from './usePullToRefresh';

function touch(type: string, el: Element, y?: number) {
  const event = new Event(type, { bubbles: true, cancelable: true });
  Object.defineProperty(event, 'touches', { value: y === undefined ? [] : [{ clientY: y }] });
  fireEvent(el, event);
  return event;
}

function list(onRefresh?: () => Promise<void>) {
  render(
    <MessageList
      title="Inbox"
      subtitle="All accounts"
      // `role` here is the folder role prop, not an ARIA role.
      // eslint-disable-next-line jsx-a11y/aria-role
      role="inbox"
      status="ready"
      items={[]}
      total={0}
      error={null}
      filtered={false}
      filter="all"
      selectedId={null}
      accounts={[]}
      now={0}
      hasMore={false}
      loadingMore={false}
      onFilter={vi.fn()}
      onSelect={vi.fn()}
      onLoadMore={vi.fn()}
      onRetry={vi.fn()}
      onRefresh={onRefresh}
    />,
  );
  return screen.getByTestId('list-scroll');
}

describe('pull to refresh', () => {
  it('checks for new mail when the list is pulled down far enough and released', async () => {
    let finish: (() => void) | undefined;
    const onRefresh = vi.fn(
      () =>
        new Promise<void>((resolve) => {
          finish = resolve;
        }),
    );
    const scroll = list(onRefresh);

    touch('touchstart', scroll, 100);
    const move = touch('touchmove', scroll, 100 + PULL_THRESHOLD * 2 + 10);
    expect(move.defaultPrevented).toBe(true);
    expect(screen.getByRole('status')).toHaveTextContent('Release to check for new mail');

    touch('touchend', scroll);
    expect(onRefresh).toHaveBeenCalledOnce();
    expect(screen.getByRole('status')).toHaveTextContent('Checking for new mail');

    await act(async () => {
      finish?.();
      await Promise.resolve();
    });
    expect(screen.getByRole('status')).toHaveTextContent('');
  });

  it('does nothing for a short pull', () => {
    const onRefresh = vi.fn(() => Promise.resolve());
    const scroll = list(onRefresh);
    touch('touchstart', scroll, 100);
    touch('touchmove', scroll, 120);
    expect(screen.getByRole('status')).toHaveTextContent('Pull to check for new mail');
    touch('touchend', scroll);
    expect(onRefresh).not.toHaveBeenCalled();
  });

  it('leaves normal scrolling alone when the list is not at the top', () => {
    const onRefresh = vi.fn(() => Promise.resolve());
    const scroll = list(onRefresh);
    scroll.scrollTop = 200;
    touch('touchstart', scroll, 100);
    const move = touch('touchmove', scroll, 300);
    expect(move.defaultPrevented).toBe(false);
    touch('touchend', scroll);
    expect(onRefresh).not.toHaveBeenCalled();
  });

  it('is off without a refresh handler (desktop)', () => {
    list();
    expect(screen.queryByRole('status')).not.toBeInTheDocument();
  });
});
