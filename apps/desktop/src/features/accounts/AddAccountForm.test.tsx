import { screen, within } from '@testing-library/react';
import { vi } from 'vitest';

import { renderApp, rows } from '../../test/renderApp';

const form = () => screen.getByRole('form', { name: 'Add an email account' });
const field = (label: string) => within(form()).getByLabelText(label);

describe('adding an account', () => {
  it('prefills suggested servers and points Gmail users to app passwords and Google sign-in', async () => {
    const { user } = renderApp({ accounts: [] });
    await screen.findByRole('heading', { name: 'Welcome to Mocky Mail' });
    await user.type(field('Email address'), 'don@gmail.com');
    await user.tab();
    expect(await within(form()).findByText(/This looks like a Google account/)).toBeInTheDocument();
    expect(within(form()).getByText(/Gmail needs an app password here/)).toBeInTheDocument();
    await user.click(within(form()).getByRole('button', { name: 'Advanced: server settings' }));
    const imap = within(form()).getByRole('group', { name: 'Incoming mail (IMAP)' });
    expect(within(imap).getByLabelText('Server')).toHaveValue('imap.gmail.com');
    expect(within(imap).getByLabelText('Port')).toHaveValue('993');
  });

  it('shows and hides the password without losing it', async () => {
    const { user } = renderApp({ accounts: [] });
    await screen.findByRole('heading', { name: 'Welcome to Mocky Mail' });
    const password = field('Password or app password');
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
    expect(await within(form()).findByText(/could not find settings for fitout.example/)).toBeInTheDocument();
    const smtp = within(form()).getByRole('group', { name: 'Outgoing mail (SMTP)' });
    expect(within(smtp).getByLabelText('Server')).toHaveValue('smtp.fitout.example');
    await user.selectOptions(within(smtp).getByLabelText('Security'), 'starttls');
    expect(within(smtp).getByLabelText('Security')).toHaveValue('starttls');
  });

  it('explains a rejected password and never keeps the password', async () => {
    const { user, api } = renderApp({ accounts: [] });
    await screen.findByRole('heading', { name: 'Welcome to Mocky Mail' });
    const spy = vi.spyOn(api, 'addAccount');
    await user.type(field('Email address'), 'don@yahoo.com');
    await user.type(field('Password or app password'), 'wrong');
    await user.click(within(form()).getByRole('button', { name: 'Add account' }));
    const alert = await within(form()).findByRole('alert');
    expect(alert).toHaveTextContent('Sign-in was not accepted');
    expect(alert).toHaveTextContent('The server rejected the username or password.');
    expect(alert).toHaveTextContent('Yahoo needs an app password');
    expect(spy).toHaveBeenCalledWith(expect.objectContaining({ email: 'don@yahoo.com', password: 'wrong' }));
    expect(field('Password or app password')).toHaveValue('');
    expect(document.body.innerHTML).not.toContain('wrong"');
  });

  it('reports connection problems with a hint', async () => {
    const { user } = renderApp({ accounts: [] });
    await screen.findByRole('heading', { name: 'Welcome to Mocky Mail' });
    await user.type(field('Email address'), 'don@nowhere.invalid');
    await user.tab();
    const imap = await within(form()).findByRole('group', { name: 'Incoming mail (IMAP)' });
    expect(within(imap).getByLabelText('Server')).toHaveValue('imap.nowhere.invalid');
    await user.type(field('Password or app password'), 'secret');
    await user.click(within(form()).getByRole('button', { name: 'Add account' }));
    const alert = await within(form()).findByRole('alert');
    expect(alert).toHaveTextContent('Could not reach the mail server');
    expect(alert).toHaveTextContent('Check your internet connection');
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
    expect(within(form()).getByRole('alert')).toHaveTextContent('Enter your email address first.');
    await user.type(field('Email address'), 'don@gmail.com');
    await user.click(within(form()).getByRole('button', { name: 'Continue with Google' }));
    const alert = await within(form()).findByRole('alert');
    expect(alert).toHaveTextContent('Not set up yet');
    expect(alert).toHaveTextContent('Google sign-in is not configured in this build yet.');
    expect(alert).toHaveTextContent('Use your email and an app password instead for now.');
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
    await user.type(field('Your name (shown to people you email)'), 'Don');
    await user.tab();
    await user.type(field('Password or app password'), 'correct horse');
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
