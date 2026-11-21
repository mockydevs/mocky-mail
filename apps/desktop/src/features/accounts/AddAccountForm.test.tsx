import { screen, within } from '@testing-library/react';
import { vi } from 'vitest';

import type { ServerSuggestion } from '../../api/types';
import { renderApp, rows } from '../../test/renderApp';

const form = () => screen.getByRole('form', { name: 'Add an email account' });
const field = (label: string) => within(form()).getByLabelText(label);

describe('adding an account', () => {
  it('prefills suggested servers and points Gmail users to app passwords and Google sign-in', async () => {
    const { user } = renderApp({ accounts: [] });
    await screen.findByRole('heading', { name: 'Welcome to Mocky Mail' });
    await user.type(field('Email address'), 'don@gmail.com');
    await user.tab();
    expect(await within(form()).findByText(/This is a Google account/)).toBeInTheDocument();
    // The suggested sign-in moves up under the email; the other provider steps aside.
    expect(within(form()).getByRole('button', { name: 'Continue with Google' })).toHaveClass('btn--primary');
    expect(within(form()).queryByRole('button', { name: 'Continue with Microsoft' })).not.toBeInTheDocument();
    expect(within(form()).getByText(/Gmail needs an app password here/)).toBeInTheDocument();
    await user.click(within(form()).getByRole('button', { name: 'Edit' }));
    const imap = within(form()).getByRole('group', { name: 'Incoming mail (IMAP)' });
    expect(within(imap).getByLabelText('Server name')).toHaveValue('imap.gmail.com');
    expect(within(imap).getByLabelText('Port')).toHaveValue('993');
  });

  it('shows and hides the password without losing it', async () => {
    const { user } = renderApp({ accounts: [] });
    await screen.findByRole('heading', { name: 'Welcome to Mocky Mail' });
    const password = field('Password');
    await user.type(password, 'abcd efgh ijkl');
    expect(password).toHaveAttribute('type', 'password');
    const toggle = within(form()).getByRole('button', { name: 'Show password' });
    expect(toggle).toHaveAttribute('aria-pressed', 'false');
    await user.click(toggle);
    expect(password).toHaveAttribute('type', 'text');
    expect(toggle).toHaveAttribute('aria-pressed', 'true');
    expect(password).toHaveValue('abcd efgh ijkl');
    await user.click(toggle);
    expect(password).toHaveAttribute('type', 'password');
  });

  it('opens manual settings with a guess when the provider is unknown', async () => {
    const { user } = renderApp({ accounts: [] });
    await screen.findByRole('heading', { name: 'Welcome to Mocky Mail' });
    await user.type(field('Email address'), 'don@fitout.example');
    await user.tab();
    expect(await within(form()).findByText(/couldn’t find settings for fitout.example/)).toBeInTheDocument();
    const smtp = within(form()).getByRole('group', { name: 'Outgoing mail (SMTP)' });
    expect(within(smtp).getByLabelText('Server name')).toHaveValue('smtp.fitout.example');
    await user.selectOptions(within(smtp).getByLabelText('Security'), 'starttls');
    expect(within(smtp).getByLabelText('Security')).toHaveValue('starttls');
  });

  it('explains a rejected password and never keeps the password', async () => {
    const { user, api } = renderApp({ accounts: [] });
    await screen.findByRole('heading', { name: 'Welcome to Mocky Mail' });
    const spy = vi.spyOn(api, 'addAccount');
    await user.type(field('Email address'), 'don@yahoo.com');
    await user.type(field('Password'), 'wrong');
    await user.click(within(form()).getByRole('button', { name: 'Add account' }));
    const alert = await within(form()).findByRole('alert');
    expect(alert).toHaveTextContent('That password wasn’t accepted');
    expect(alert).toHaveTextContent('Yahoo needs an app password');
    expect(spy).toHaveBeenCalledWith(expect.objectContaining({ email: 'don@yahoo.com', password: 'wrong' }));
    expect(field('Password')).toHaveValue('');
    expect(document.body.innerHTML).not.toContain('wrong"');
  });

  it('reports connection problems with a hint', async () => {
    const { user } = renderApp({ accounts: [] });
    await screen.findByRole('heading', { name: 'Welcome to Mocky Mail' });
    await user.type(field('Email address'), 'don@nowhere.invalid');
    await user.tab();
    const imap = await within(form()).findByRole('group', { name: 'Incoming mail (IMAP)' });
    expect(within(imap).getByLabelText('Server name')).toHaveValue('imap.nowhere.invalid');
    await user.type(field('Password'), 'secret');
    await user.click(within(form()).getByRole('button', { name: 'Add account' }));
    const alert = await within(form()).findByRole('alert');
    expect(alert).toHaveTextContent('We couldn’t reach the mail server');
    expect(alert).toHaveTextContent('Check your internet connection');
  });

  it('waits for the server lookup instead of adding the account with a guess', async () => {
    const { user, api } = renderApp({ accounts: [] });
    await screen.findByRole('heading', { name: 'Welcome to Mocky Mail' });
    // The lookup is still under way when Add account is pressed.
    let answer: (found: ServerSuggestion | null) => void = () => undefined;
    vi.spyOn(api, 'suggestServers').mockReturnValue(
      new Promise((resolve) => {
        answer = resolve;
      }),
    );
    const add = vi.spyOn(api, 'addAccount');
    await user.type(field('Email address'), 'info@clinic.example');
    await user.type(field('Password'), 'secret');
    expect(within(form()).getByRole('status')).toHaveTextContent('Finding your mail settings');
    await user.click(within(form()).getByRole('button', { name: 'Add account' }));
    expect(add).not.toHaveBeenCalled();
    answer({
      imap: { host: 'mail.host.example', port: 993, security: 'tls' },
      smtp: { host: 'mail.host.example', port: 587, security: 'starttls' },
      auth: 'password',
    });
    await vi.waitFor(() => {
      expect(add).toHaveBeenCalledWith(
        expect.objectContaining({
          imap: { host: 'mail.host.example', port: 993, security: 'tls' },
          smtp: { host: 'mail.host.example', port: 587, security: 'starttls' },
        }),
      );
    });
  });

  it('replaces the servers when the address moves to another domain, unless they were edited', async () => {
    const { user } = renderApp({ accounts: [] });
    await screen.findByRole('heading', { name: 'Welcome to Mocky Mail' });
    await user.type(field('Email address'), 'don@yahoo.com');
    await user.tab();
    expect(await within(form()).findByText(/Incoming imap.mail.yahoo.com:993/)).toBeInTheDocument();
    await user.clear(field('Email address'));
    await user.type(field('Email address'), 'don@mocky.co.ke');
    await user.tab();
    expect(await within(form()).findByText(/Incoming mail.mailer.co.ke/)).toBeInTheDocument();

    // Settings typed by hand stay put when the address changes again.
    await user.click(within(form()).getByRole('button', { name: 'Edit' }));
    const imap = within(form()).getByRole('group', { name: 'Incoming mail (IMAP)' });
    await user.clear(within(imap).getByLabelText('Server name'));
    await user.type(within(imap).getByLabelText('Server name'), 'imap.mine.example');
    await user.clear(field('Email address'));
    await user.type(field('Email address'), 'don@yahoo.com');
    await user.tab();
    expect(within(imap).getByLabelText('Server name')).toHaveValue('imap.mine.example');
  });

  it('keeps the suggested sign-in while the name part of the address is edited', async () => {
    const { user } = renderApp({ accounts: [] });
    await screen.findByRole('heading', { name: 'Welcome to Mocky Mail' });
    await user.type(field('Email address'), 'don@gmail.com');
    await user.tab();
    expect(await within(form()).findByText(/This is a Google account/)).toBeInTheDocument();
    await user.type(field('Email address'), '{Home}x');
    expect(field('Email address')).toHaveValue('xdon@gmail.com');
    expect(within(form()).getByText(/This is a Google account/)).toBeInTheDocument();
  });

  it('says the server name is wrong when its certificate is for another name', async () => {
    const { user, api } = renderApp({ accounts: [] });
    await screen.findByRole('heading', { name: 'Welcome to Mocky Mail' });
    vi.spyOn(api, 'addAccount').mockRejectedValue({
      kind: 'tls',
      message: "couldn't make a secure connection to mail.clinic.example: invalid peer certificate: NotValidForName",
    });
    await user.type(field('Email address'), 'don@mocky.co.ke');
    await user.type(field('Password'), 'secret');
    await user.click(within(form()).getByRole('button', { name: 'Add account' }));
    const alert = await within(form()).findByRole('alert');
    expect(alert).toHaveTextContent('The server name looks wrong for this address');
    // The server fields open so the name can be corrected.
    expect(within(form()).getByRole('group', { name: 'Incoming mail (IMAP)' })).toBeInTheDocument();
  });

  it('requires an email and a password before calling the backend', async () => {
    const { user, api } = renderApp({ accounts: [] });
    await screen.findByRole('heading', { name: 'Welcome to Mocky Mail' });
    const spy = vi.spyOn(api, 'addAccount');
    await user.click(within(form()).getByRole('button', { name: 'Add account' }));
    expect(within(form()).getByRole('alert')).toHaveTextContent('Enter a valid email address.');
    await user.type(field('Email address'), 'don@mocky.co.ke');
    await user.click(within(form()).getByRole('button', { name: 'Add account' }));
    expect(within(form()).getByRole('alert')).toHaveTextContent('Enter your password or app password.');
    expect(spy).not.toHaveBeenCalled();
  });

  it('shows a clear message when Google sign-in is not configured yet', async () => {
    const { user } = renderApp({ accounts: [] });
    await screen.findByRole('heading', { name: 'Welcome to Mocky Mail' });
    await user.click(within(form()).getByRole('button', { name: 'Continue with Google' }));
    const alert = await within(form()).findByRole('alert');
    expect(alert).toHaveTextContent('Not set up yet');
    expect(alert).toHaveTextContent('Google sign-in is not configured in this build yet.');
    expect(alert).toHaveTextContent('Use your email and an app password instead for now.');
  });

  it('signs in with Google without typing an address first', async () => {
    const { user, api } = renderApp({ accounts: [], oauthConfigured: true });
    await screen.findByRole('heading', { name: 'Welcome to Mocky Mail' });
    const spy = vi.spyOn(api, 'addOAuthAccount');
    // Sign-in comes before the email field, so it's the first thing offered.
    const google = within(form()).getByRole('button', { name: 'Continue with Google' });
    expect(google.compareDocumentPosition(field('Email address')) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    await user.click(google);
    expect(spy).toHaveBeenCalledWith(expect.objectContaining({ email: '', provider: 'google' }));
    expect(await screen.findByRole('list', { name: 'All accounts' })).toBeInTheDocument();
  });

  it('shows that sign-in is under way until the provider answers', async () => {
    const { user, api } = renderApp({ accounts: [], oauthConfigured: true });
    await screen.findByRole('heading', { name: 'Welcome to Mocky Mail' });
    vi.spyOn(api, 'addOAuthAccount').mockReturnValue(new Promise(() => undefined));
    await user.click(within(form()).getByRole('button', { name: 'Continue with Google' }));
    expect(within(form()).getByRole('status')).toHaveTextContent('Signing in with Google');
    expect(within(form()).getByRole('button', { name: 'Waiting for Google' })).toBeDisabled();
  });

  it('asks to fix a half-typed address instead of sending it as a hint', async () => {
    const { user, api } = renderApp({ accounts: [], oauthConfigured: true });
    await screen.findByRole('heading', { name: 'Welcome to Mocky Mail' });
    const spy = vi.spyOn(api, 'addOAuthAccount');
    await user.type(field('Email address'), 'don@');
    await user.click(within(form()).getByRole('button', { name: 'Continue with Google' }));
    expect(within(form()).getByRole('alert')).toHaveTextContent('clear it to choose an account');
    expect(spy).not.toHaveBeenCalled();
  });

  it('signs in with Microsoft when OAuth is configured', async () => {
    const { user } = renderApp({ accounts: [], oauthConfigured: true });
    await screen.findByRole('heading', { name: 'Welcome to Mocky Mail' });
    await user.type(field('Email address'), 'don@outlook.com');
    await user.click(within(form()).getByRole('button', { name: 'Continue with Microsoft' }));
    expect(await screen.findByRole('list', { name: 'All accounts' })).toBeInTheDocument();
    expect(screen.getByRole('list', { name: 'don@outlook.com' })).toBeInTheDocument();
  });

  it('adds a password account and opens the mailbox', async () => {
    const { user } = renderApp({ accounts: [] });
    await screen.findByRole('heading', { name: 'Welcome to Mocky Mail' });
    await user.type(field('Email address'), 'don@mocky.co.ke');
    await user.type(field('Your name (optional)'), 'Don');
    await user.tab();
    await user.type(field('Password'), 'correct horse');
    await user.click(within(form()).getByRole('button', { name: 'Add account' }));
    expect(await screen.findByRole('list', { name: 'Don' })).toBeInTheDocument();
    expect(await screen.findByText('All caught up')).toBeInTheDocument();
    expect(document.body.innerHTML).not.toContain('correct horse');
  });

  it('can add a second account from the sidebar', async () => {
    const { user } = renderApp();
    await rows();
    await user.click(screen.getByRole('button', { name: 'Add account' }));
    expect(screen.getByRole('form', { name: 'Add an email account' })).toBeInTheDocument();
    await user.click(within(form()).getByRole('button', { name: 'Cancel' }));
    expect(await rows()).toBeDefined();
  });
});
