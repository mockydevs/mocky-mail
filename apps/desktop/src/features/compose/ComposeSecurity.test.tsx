import { screen, waitFor, within, type BoundFunctions, type queries } from '@testing-library/react';
import { afterEach, vi } from 'vitest';

import { MOCK_SETTINGS } from '../../api/mock/mockData';
import type { MockOptions } from '../../api/mock/mockMailApi';
import type { Draft } from '../../api/types';
import { renderApp, rows } from '../../test/renderApp';
import { preloadEditor } from './editor/loader';

beforeAll(async () => {
  await preloadEditor();
});

const SLOW = { timeout: 3000 };

async function newMessage(options: MockOptions = {}) {
  const ctx = renderApp(options);
  await rows();
  const send = vi.spyOn(ctx.api, 'sendMessage');
  const keys = vi.spyOn(ctx.api, 'getAccountKeys');
  await ctx.user.click(screen.getByRole('button', { name: 'New message' }));
  const form = within(screen.getByRole('form', { name: 'New message' }));
  await form.findByLabelText('Message');
  // What the From account can sign with is known.
  await waitFor(() => {
    expect(keys).toHaveBeenCalled();
  });
  return { ...ctx, send, keys, form };
}

/** Open Options and hand back the menu. */
async function options(ctx: Awaited<ReturnType<typeof newMessage>>) {
  await ctx.user.click(ctx.form.getByRole('button', { name: 'Options' }));
  return within(screen.getByRole('menu'));
}

/** A Sign or Encrypt item, once the account's keys are known. */
async function enabledItem(menu: BoundFunctions<typeof queries>, name: string): Promise<HTMLElement> {
  return waitFor(() => {
    const el = menu.getByRole('menuitemcheckbox', { name });
    expect(el).toBeEnabled();
    return el;
  });
}

const sent = (send: { mock: { calls: unknown[][] } }): Draft => {
  const [draft] = send.mock.calls[0] ?? [];
  if (!draft) throw new Error('nothing was sent');
  return draft as Draft;
};

describe('Sign and Encrypt in compose', () => {
  it('signs with the account’s certificate when Sign is ticked', async () => {
    const ctx = await newMessage();
    let menu = await options(ctx);
    const sign = await menu.findByRole('menuitemcheckbox', { name: 'Sign' });
    await waitFor(() => {
      expect(sign).toBeEnabled();
    });
    expect(sign).toHaveAttribute('aria-checked', 'false');
    expect(menu.getByRole('menuitemcheckbox', { name: 'Request a read receipt' })).toBeInTheDocument();
    await ctx.user.click(sign);
    expect(
      ctx.form.getByText('Signed with your S/MIME certificate, so recipients can check it came from you and wasn’t changed.'),
    ).toBeInTheDocument();
    menu = await options(ctx);
    expect(menu.getByRole('menuitemcheckbox', { name: 'Sign' })).toHaveAttribute('aria-checked', 'true');
    expect(menu.getByRole('menuitemcheckbox', { name: 'Encrypt' })).toHaveAttribute('aria-checked', 'false');
    await ctx.user.keyboard('{Escape}');

    await ctx.user.type(ctx.form.getByLabelText('To'), 'peter@client.example');
    await ctx.user.click(ctx.form.getByRole('button', { name: 'Send' }));
    await waitFor(() => {
      expect(ctx.send).toHaveBeenCalled();
    });
    expect(sent(ctx.send)).toMatchObject({ sign: true, securityKind: 'smime' });
    expect(sent(ctx.send).encrypt).not.toBe(true);
  });

  it('offers S/MIME or OpenPGP when the account has both', async () => {
    const ctx = await newMessage();
    let menu = await options(ctx);
    const group = within(await menu.findByRole('group', { name: 'Sign and encrypt with' }));
    expect(group.getByRole('menuitemradio', { name: 'S/MIME certificate' })).toHaveAttribute('aria-checked', 'true');
    await ctx.user.click(group.getByRole('menuitemradio', { name: 'OpenPGP key' }));
    menu = await options(ctx);
    expect(menu.getByRole('menuitemradio', { name: 'OpenPGP key' })).toHaveAttribute('aria-checked', 'true');
    await ctx.user.click(menu.getByRole('menuitemcheckbox', { name: 'Sign' }));
    expect(ctx.form.getByText(/Signed with your OpenPGP key/)).toBeInTheDocument();
    await ctx.user.type(ctx.form.getByLabelText('To'), 'amina@studio.example');
    await ctx.user.click(ctx.form.getByRole('button', { name: 'Send' }));
    await waitFor(() => {
      expect(ctx.send).toHaveBeenCalled();
    });
    expect(sent(ctx.send)).toMatchObject({ sign: true, securityKind: 'openpgp' });
  });

  it('uses the only kind an account has, without asking', async () => {
    const ctx = await newMessage();
    await ctx.user.selectOptions(ctx.form.getByLabelText('From'), 'personal');
    await waitFor(() => {
      expect(ctx.keys).toHaveBeenCalledWith('personal');
    });
    const menu = await options(ctx);
    await waitFor(() => {
      expect(menu.queryByRole('group', { name: 'Sign and encrypt with' })).not.toBeInTheDocument();
    });
    const sign = await enabledItem(menu, 'Sign');
    await ctx.user.click(sign);
    expect(ctx.form.getByText(/Signed with your OpenPGP key/)).toBeInTheDocument();
    await ctx.user.type(ctx.form.getByLabelText('To'), 'brian@personal.example');
    await ctx.user.click(ctx.form.getByRole('button', { name: 'Send' }));
    await waitFor(() => {
      expect(ctx.send).toHaveBeenCalled();
    });
    expect(sent(ctx.send)).toMatchObject({ accountId: 'personal', sign: true, securityKind: 'openpgp' });
  });

  it('explains why Sign and Encrypt are off when the account has no key, even with the defaults on', async () => {
    const ctx = await newMessage({
      accountKeys: {},
      settings: { ...MOCK_SETTINGS, signByDefault: true, encryptWhenPossible: true },
    });
    const menu = await options(ctx);
    const sign = menu.getByRole('menuitemcheckbox', { name: /^Sign/ });
    const encrypt = menu.getByRole('menuitemcheckbox', { name: /^Encrypt/ });
    for (const item of [sign, encrypt]) {
      expect(item).toBeDisabled();
      expect(item).toHaveAttribute('aria-checked', 'false');
      expect(item).toHaveTextContent('Needs a certificate or key for this account, in Settings › Security');
    }
    expect(menu.queryByRole('group', { name: 'Sign and encrypt with' })).not.toBeInTheDocument();
    expect(ctx.form.queryByText(/Signed with/)).not.toBeInTheDocument();
  });

  it('signs by default when Settings say so, without counting it as a change', async () => {
    const ctx = await newMessage({ settings: { ...MOCK_SETTINGS, signByDefault: true } });
    expect(await ctx.form.findByText(/Signed with your S\/MIME certificate/)).toBeInTheDocument();
    const menu = await options(ctx);
    expect(menu.getByRole('menuitemcheckbox', { name: 'Sign' })).toHaveAttribute('aria-checked', 'true');
    await ctx.user.keyboard('{Escape}');
    // Nothing typed: closing doesn't ask about unsaved changes.
    await ctx.user.click(ctx.form.getByRole('button', { name: 'Close' }));
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(screen.queryByRole('form', { name: 'New message' })).not.toBeInTheDocument();
  });

  it('with Encrypt when possible, encrypts once every recipient has a key and stops when one has none', async () => {
    const ctx = await newMessage({ settings: { ...MOCK_SETTINGS, encryptWhenPossible: true } });
    const check = vi.spyOn(ctx.api, 'checkEncryption');
    await ctx.user.type(ctx.form.getByLabelText('To'), 'wanjiru@fitout.example,');
    expect(await ctx.form.findByText('Encrypted with S/MIME. Only the recipients can read it; the subject stays visible.', {}, SLOW)).toBeInTheDocument();
    expect(check).toHaveBeenLastCalledWith('work', ['wanjiru@fitout.example'], 'smime');

    await ctx.user.type(ctx.form.getByLabelText('To'), 'nobody@else.example,');
    await waitFor(() => {
      expect(ctx.form.queryByText('Encrypted with S/MIME. Only the recipients can read it; the subject stays visible.')).not.toBeInTheDocument();
    }, SLOW);
    // Off by itself, so nobody is marked.
    expect(ctx.form.queryByText(/Can’t encrypt to/)).not.toBeInTheDocument();
    const menu = await options(ctx);
    expect(menu.getByRole('menuitemcheckbox', { name: 'Encrypt' })).toHaveAttribute('aria-checked', 'false');
    await ctx.user.keyboard('{Escape}');
    await ctx.user.click(ctx.form.getByRole('button', { name: 'Send' }));
    await waitFor(() => {
      expect(ctx.send).toHaveBeenCalled();
    });
    expect(sent(ctx.send).encrypt).not.toBe(true);
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('leaves Encrypt alone once it was chosen by hand', async () => {
    const ctx = await newMessage({ settings: { ...MOCK_SETTINGS, encryptWhenPossible: true } });
    const menu = await options(ctx);
    // Switched on, then off again, before any recipient.
    await ctx.user.click(await enabledItem(menu, 'Encrypt'));
    await ctx.user.click((await options(ctx)).getByRole('menuitemcheckbox', { name: 'Encrypt' }));
    const check = vi.spyOn(ctx.api, 'checkEncryption');
    await ctx.user.type(ctx.form.getByLabelText('To'), 'wanjiru@fitout.example,');
    await new Promise((r) => setTimeout(r, 700));
    expect(check).not.toHaveBeenCalled();
    expect(ctx.form.queryByText(/Encrypted with/)).not.toBeInTheDocument();
  });
});

/** Encrypt on, writing to someone with a certificate and someone without. */
async function encryptingToSomeoneWithoutKey(options_: MockOptions = {}, to = 'wanjiru@fitout.example, nobody@else.example,') {
  const ctx = await newMessage(options_);
  const menu = await options(ctx);
  await ctx.user.click(await enabledItem(menu, 'Encrypt'));
  expect(ctx.form.getByText('Encrypted with S/MIME. Only the recipients can read it; the subject stays visible.')).toBeInTheDocument();
  await ctx.user.type(ctx.form.getByLabelText('To'), to);
  await ctx.user.type(ctx.form.getByLabelText('Subject'), 'Door codes');
  return ctx;
}

describe('Encrypt with recipients that have no key', () => {
  it('marks them in red as the recipients change', async () => {
    const ctx = await encryptingToSomeoneWithoutKey();
    expect(
      await ctx.form.findByText('Can’t encrypt to nobody@else.example: there’s no certificate for them.', {}, SLOW),
    ).toBeInTheDocument();
    expect(ctx.form.getByRole('button', { name: 'Remove nobody@else.example' }).closest('li')).toHaveClass('chip--bad');
    expect(ctx.form.getByRole('button', { name: 'Remove wanjiru@fitout.example' }).closest('li')).not.toHaveClass('chip--bad');
    expect(ctx.form.getByRole('button', { name: 'Remove nobody@else.example' }).closest('li')).toHaveTextContent(
      'can’t encrypt to them',
    );

    // Taking them out clears the warning.
    await ctx.user.click(ctx.form.getByRole('button', { name: 'Remove nobody@else.example' }));
    await waitFor(() => {
      expect(ctx.form.queryByText(/Can’t encrypt to/)).not.toBeInTheDocument();
    });
  });

  it('on Send, can take them out and send encrypted to the rest', async () => {
    const ctx = await encryptingToSomeoneWithoutKey();
    await ctx.user.click(ctx.form.getByRole('button', { name: 'Send' }));
    const dialog = within(await screen.findByRole('dialog', { name: 'Can’t encrypt to everyone' }));
    expect(screen.getByRole('dialog')).toHaveAccessibleDescription(
      'There’s no certificate for nobody@else.example, so they couldn’t read an encrypted message.',
    );
    expect(ctx.send).not.toHaveBeenCalled();
    await ctx.user.click(dialog.getByRole('button', { name: 'Remove nobody@else.example and send encrypted' }));
    await waitFor(() => {
      expect(ctx.send).toHaveBeenCalledTimes(1);
    });
    expect(sent(ctx.send)).toMatchObject({ to: 'wanjiru@fitout.example', encrypt: true, securityKind: 'smime' });
    expect(screen.queryByRole('form', { name: 'New message' })).not.toBeInTheDocument();
  });

  it('on Send, can send without encryption to everyone', async () => {
    const ctx = await encryptingToSomeoneWithoutKey();
    await ctx.user.click(ctx.form.getByRole('button', { name: 'Send' }));
    const dialog = within(await screen.findByRole('dialog', { name: 'Can’t encrypt to everyone' }));
    await ctx.user.click(dialog.getByRole('button', { name: 'Send without encryption' }));
    await waitFor(() => {
      expect(ctx.send).toHaveBeenCalledTimes(1);
    });
    expect(sent(ctx.send)).toMatchObject({ to: 'wanjiru@fitout.example, nobody@else.example', encrypt: false });
  });

  it('on Send, can cancel and keep writing', async () => {
    const ctx = await encryptingToSomeoneWithoutKey();
    await ctx.user.click(ctx.form.getByRole('button', { name: 'Send' }));
    const dialog = within(await screen.findByRole('dialog', { name: 'Can’t encrypt to everyone' }));
    await ctx.user.click(dialog.getByRole('button', { name: 'Cancel' }));
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(ctx.send).not.toHaveBeenCalled();
    expect(ctx.form.getByRole('button', { name: 'Remove nobody@else.example' })).toBeInTheDocument();
    expect(ctx.form.getByRole('button', { name: 'Send' })).toBeEnabled();
    const menu = await options(ctx);
    expect(menu.getByRole('menuitemcheckbox', { name: 'Encrypt' })).toHaveAttribute('aria-checked', 'true');
  });

  it('has nobody to take out when no recipient has a key', async () => {
    const ctx = await encryptingToSomeoneWithoutKey({}, 'nobody@else.example,');
    await ctx.user.click(ctx.form.getByRole('button', { name: 'Send' }));
    const dialog = within(await screen.findByRole('dialog', { name: 'Can’t encrypt to everyone' }));
    expect(dialog.getByText(/Nobody you’re writing to can get encrypted mail yet/)).toBeInTheDocument();
    expect(dialog.queryByRole('button', { name: /^Remove/ })).not.toBeInTheDocument();
    expect(dialog.getByRole('button', { name: 'Send without encryption' })).toBeInTheDocument();
  });

  it('sends encrypted straight away when everyone has a key', async () => {
    const ctx = await encryptingToSomeoneWithoutKey({}, 'wanjiru@fitout.example, peter@client.example,');
    await ctx.user.click(ctx.form.getByRole('button', { name: 'Send' }));
    await waitFor(() => {
      expect(ctx.send).toHaveBeenCalledTimes(1);
    });
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(sent(ctx.send)).toMatchObject({ encrypt: true, securityKind: 'smime' });
  });
});

describe('on a phone', () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('has the same Sign and Encrypt options, and asks the same question on Send', async () => {
    vi.stubGlobal(
      'matchMedia',
      vi.fn((query: string) => ({ matches: query.startsWith('(max-width'), media: query, addEventListener: vi.fn(), removeEventListener: vi.fn() })),
    );
    const ctx = await encryptingToSomeoneWithoutKey({ phone: true });
    expect(ctx.form.getByRole('toolbar', { name: 'Formatting' }).closest('.rte')).toHaveClass('rte--phone');
    const menu = await options(ctx);
    expect(menu.getByRole('menuitemcheckbox', { name: 'Sign' })).toBeEnabled();
    expect(menu.getByRole('menuitemcheckbox', { name: 'Encrypt' })).toHaveAttribute('aria-checked', 'true');
    await ctx.user.keyboard('{Escape}');
    await ctx.user.click(ctx.form.getByRole('button', { name: 'Send' }));
    const dialog = within(await screen.findByRole('dialog', { name: 'Can’t encrypt to everyone' }));
    await ctx.user.click(dialog.getByRole('button', { name: 'Remove nobody@else.example and send encrypted' }));
    await waitFor(() => {
      expect(ctx.send).toHaveBeenCalledTimes(1);
    });
    expect(sent(ctx.send)).toMatchObject({ to: 'wanjiru@fitout.example', encrypt: true });
  });
});
