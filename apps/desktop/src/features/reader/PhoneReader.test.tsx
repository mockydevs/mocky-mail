import { act, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeEach, vi } from 'vitest';

import { reader, renderApp, rows } from '../../test/renderApp';

/** Pretend the window is phone-sized. */
function phoneWidth() {
  vi.stubGlobal(
    'matchMedia',
    vi.fn((query: string) => ({
      matches: query === '(max-width: 760px)',
      media: query,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
    })),
  );
}

const app = () => document.querySelector('.app');

describe('phone message view', () => {
  beforeEach(() => {
    phoneWidth();
  });
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('opens a message full screen with a compact header, then goes back to the list', async () => {
    const { user } = renderApp();
    const list = await rows();
    expect(app()).not.toHaveClass('app--reading');
    await user.click(list.getByRole('button', { name: /Build server/ }));
    expect(app()).toHaveClass('app--reading');

    const pane = within(reader());
    await pane.findByTitle('Message: Nightly build passed on main');
    // No desktop toolbar: one bar on top, reply actions at the bottom.
    expect(pane.queryByRole('toolbar', { name: 'Message actions' })).toHaveClass('phone-bar');
    expect(pane.getByRole('toolbar', { name: 'Reply' })).toBeInTheDocument();
    // Images notice is one slim line.
    expect(pane.getByRole('button', { name: 'Show images (1 blocked)' })).toHaveTextContent('Show');

    // Recipients and the account stay behind the date until asked for.
    expect(pane.queryByText(/^To /)).not.toBeInTheDocument();
    const more = pane.getByRole('button', { name: /^to / });
    await user.click(more);
    expect(more).toHaveAttribute('aria-expanded', 'true');
    expect(pane.getByText(/^To /)).toBeInTheDocument();

    // The email is part of the page scroll, not a box of its own.
    expect(pane.getByTitle('Message: Nightly build passed on main').style.flexGrow).toBe('0');

    // Back goes through the browser history, so the phone's Back button works the same way.
    await user.click(pane.getByRole('button', { name: 'Back to list' }));
    await waitFor(() => {
      expect(app()).not.toHaveClass('app--reading');
    });
  });

  it('closes the message with the phone Back button', async () => {
    const { user } = renderApp();
    await user.click((await rows()).getByRole('button', { name: /Build server/ }));
    expect(app()).toHaveClass('app--reading');
    act(() => {
      window.dispatchEvent(new PopStateEvent('popstate'));
    });
    expect(app()).not.toHaveClass('app--reading');
  });

  it('replies in a panel at the bottom, with a way to the full editor', async () => {
    const { user } = renderApp();
    await user.click((await rows()).getByRole('button', { name: /Wanjiru/ }));
    const pane = within(reader());
    await user.click(await pane.findByRole('button', { name: 'Reply' }));
    const box = pane.getByLabelText('Quick reply to Wanjiru');
    expect(box).toHaveFocus();
    expect(pane.queryByRole('toolbar', { name: 'Reply' })).not.toBeInTheDocument();
    await user.type(box, 'Approved.');
    await user.click(pane.getByRole('button', { name: 'Send' }));
    expect(await pane.findByText('Reply queued in the Outbox.')).toBeInTheDocument();

    await user.click(pane.getByRole('button', { name: 'Close reply' }));
    expect(pane.getByRole('toolbar', { name: 'Reply' })).toBeInTheDocument();
    await user.click(pane.getByRole('button', { name: 'Reply' }));
    await user.click(pane.getByRole('button', { name: 'Full editor' }));
    expect(await screen.findByRole('heading', { name: 'Reply' })).toBeInTheDocument();
  });
});
