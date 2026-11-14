import { vi } from 'vitest';
import { screen, within } from '@testing-library/react';

import { MOCK_MESSAGES } from '../../api/mock/mockData';
import { reader, renderApp, rows } from '../../test/renderApp';

describe('reading pane', () => {
  it('blocks remote images until asked, then reloads with them', async () => {
    const { user } = renderApp();
    await user.click((await rows()).getByRole('button', { name: /Build server/ }));
    const frame = await within(reader()).findByTitle('Message: Nightly build passed on main');
    expect(frame).toHaveAttribute('sandbox', 'allow-popups');
    expect(frame.getAttribute('srcdoc')).not.toContain('badge.svg');
    await user.click(within(reader()).getByRole('button', { name: 'Show images (1 blocked)' }));
    const shown = await within(reader()).findByTitle('Message: Nightly build passed on main');
    await vi.waitFor(() => {
      expect(shown.getAttribute('srcdoc')).toContain('https://ci.example/badge.svg');
    });
    expect(within(reader()).queryByRole('button', { name: /Show images/ })).not.toBeInTheDocument();
  });

  it('shows HTML mail dark in dark mode, with a per-email switch back to original colours', async () => {
    const { user } = renderApp({ settings: { theme: 'dark', remoteImages: 'block', notifications: false, sound: false } });
    await user.click((await rows()).getByRole('button', { name: /Build server/ }));
    const frame = await within(reader()).findByTitle('Message: Nightly build passed on main');
    expect(frame.getAttribute('srcdoc')).toContain('invert(1)');

    const toggle = within(reader()).getByRole('button', { name: 'Show original colours' });
    expect(toggle).toHaveAttribute('aria-pressed', 'false');
    await user.click(toggle);
    expect(toggle).toHaveAttribute('aria-pressed', 'true');
    expect(within(reader()).getByTitle('Message: Nightly build passed on main').getAttribute('srcdoc')).not.toContain('invert(');

    // The choice is per email: the next HTML email opens dark again.
    await user.click((await rows()).getByRole('button', { name: /Wanjiru/ }));
    await user.click((await rows()).getByRole('button', { name: /Build server/ }));
    const again = await within(reader()).findByTitle('Message: Nightly build passed on main');
    expect(again.getAttribute('srcdoc')).toContain('invert(1)');
  });

  it('has no colours switch in light mode', async () => {
    const { user } = renderApp({ settings: { theme: 'light', remoteImages: 'block', notifications: false, sound: false } });
    await user.click((await rows()).getByRole('button', { name: /Build server/ }));
    await within(reader()).findByTitle('Message: Nightly build passed on main');
    expect(within(reader()).queryByRole('button', { name: 'Show original colours' })).not.toBeInTheDocument();
  });

  it('gives HTML mail the rest of the pane, with attachments above and quick reply below', async () => {
    const { user } = renderApp({
      messages: MOCK_MESSAGES.map((m) =>
        m.id === 'm1' ? { ...m, html: '<p>Quote</p>', text: null } : m,
      ),
    });
    await rows();
    const frame = await within(reader()).findByTitle(/Message: Revised quotation/);
    const holder = frame.parentElement;
    if (!holder) throw new Error('frame has no container');
    expect(holder).toHaveStyle({ display: 'flex', flexDirection: 'column', minHeight: '0px' });
    expect(holder.style.flexGrow).toBe('1');
    expect(holder.parentElement).toBe(reader());
    expect(frame.style.flexGrow).toBe('1');
    expect(frame.style.height).toBe('');

    const attachments = within(reader()).getByRole('list', { name: 'Attachments' });
    const quickReply = within(reader()).getByLabelText('Quick reply to Wanjiru');
    // Attachments come before the frame; quick reply after it.
    expect(attachments.compareDocumentPosition(frame) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(frame.compareDocumentPosition(quickReply) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    await user.type(quickReply, 'Signed, see attached.');
    await user.click(within(reader()).getByRole('button', { name: 'Send' }));
    expect(await within(reader()).findByText('Reply queued in the Outbox.')).toBeInTheDocument();
  });

  it('opens links in plain text mail through the system browser', async () => {
    const { user, api } = renderApp();
    await rows();
    const link = await within(reader()).findByRole('link', { name: 'https://fitout.example/q/2041' });
    await user.click(link);
    expect(api.opened).toEqual(['https://fitout.example/q/2041']);
  });

  it('saves attachments and reports where', async () => {
    const { user, api } = renderApp();
    await rows();
    await user.click(await within(reader()).findByRole('button', { name: 'Save Quotation-Westlands-rev2.pdf' }));
    expect(await within(reader()).findByText('Saved to /home/you/Downloads/Quotation-Westlands-rev2.pdf')).toBeInTheDocument();
    api.failNext('saveAttachment', { kind: 'storage', message: 'No space left on the disk.' });
    await user.click(within(reader()).getByRole('button', { name: 'Save Floor-plan.png' }));
    expect(await within(reader()).findByRole('alert')).toHaveTextContent('No space left on the disk.');
  });

  it('shows Cc and Reply-To', async () => {
    const { user } = renderApp();
    await rows();
    expect(await within(reader()).findByText(/Cc Amina Hassan <amina@studio.example>/)).toBeInTheDocument();
    await user.click((await rows()).getByRole('button', { name: /Brian Otieno/ }));
    expect(await within(reader()).findByText(/Replies go to Brian \(phone\)/)).toBeInTheDocument();
  });

  it('marks read and unread from the toolbar', async () => {
    const { user } = renderApp();
    await rows();
    await user.click(await within(reader()).findByRole('button', { name: 'Mark as read' }));
    expect(await within(reader()).findByRole('button', { name: 'Mark as unread' })).toBeInTheDocument();
    expect(screen.getAllByRole('button', { name: /Inbox 3 unread/ }).length).toBeGreaterThan(0);
  });
});
