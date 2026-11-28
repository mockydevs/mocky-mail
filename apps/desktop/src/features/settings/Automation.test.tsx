import { screen, within } from '@testing-library/react';
import { vi } from 'vitest';

import { MOCK_SETTINGS } from '../../api/mock/mockData';
import { renderApp, rows } from '../../test/renderApp';
import { AUTOMATION_DOCS } from './AutomationSection';

async function openAutomation(options: Parameters<typeof renderApp>[0] = {}) {
  const ctx = renderApp(options);
  await rows();
  await ctx.user.click(screen.getByRole('button', { name: 'Settings' }));
  const settings = within(screen.getByRole('region', { name: 'Settings' }));
  return { ...ctx, section: within(settings.getByRole('region', { name: 'Automation' })) };
}

describe('Settings › Automation', () => {
  it('explains what it allows and is off to begin with', async () => {
    const { section, api } = await openAutomation();
    expect(section.getByText(/Let programs on this computer, like the mocky-mail command line or AI tools that speak MCP/)).toBeInTheDocument();
    expect(section.getByRole('radio', { name: /^Off/ })).toBeChecked();
    expect(section.queryByLabelText('MCP configuration')).not.toBeInTheDocument();
    expect(section.queryByRole('button', { name: 'Reset secret' })).not.toBeInTheDocument();
    expect((await api.getSettings()).automation).toBe('off');
  });

  it('turns on look only, then full access with a warning, and shows how to connect', async () => {
    const { user, api, section } = await openAutomation();
    await user.click(section.getByRole('radio', { name: /^Look only/ }));
    expect((await api.getSettings()).automation).toBe('read');
    const mcp = await section.findByLabelText('MCP configuration');
    expect(mcp).toHaveAttribute('readonly');
    expect(JSON.parse((mcp as HTMLTextAreaElement).value)).toEqual({
      mcpServers: { 'mocky-mail': { command: 'C:\\Program Files\\Mocky Mail\\mocky-mail.exe', args: ['mcp'] } },
    });
    expect(section.getByText('C:\\Program Files\\Mocky Mail\\mocky-mail.exe')).toBeInTheDocument();
    expect(section.getByText(/automation-token/)).toBeInTheDocument();
    expect(section.queryByText('Full access lets programs send, move and delete mail')).not.toBeInTheDocument();

    await user.click(section.getByRole('radio', { name: /^Full access/ }));
    expect((await api.getSettings()).automation).toBe('full');
    expect(section.getByRole('note')).toHaveTextContent('Full access lets programs send, move and delete mail');

    await user.click(section.getByRole('radio', { name: /^Off/ }));
    expect((await api.getSettings()).automation).toBe('off');
    expect(section.queryByLabelText('MCP configuration')).not.toBeInTheDocument();
  });

  it('copies the command-line path and the MCP configuration', async () => {
    const { user, section } = await openAutomation({ settings: { ...MOCK_SETTINGS, automation: 'read' } });
    await user.click(await section.findByRole('button', { name: 'Copy path' }));
    expect(await navigator.clipboard.readText()).toBe('C:\\Program Files\\Mocky Mail\\mocky-mail.exe');
    expect(section.getByRole('button', { name: 'Copied' })).toBeInTheDocument();

    await user.click(section.getByRole('button', { name: 'Copy' }));
    expect(JSON.parse(await navigator.clipboard.readText())).toHaveProperty('mcpServers.mocky-mail.args', ['mcp']);
  });

  it('resets the secret only after asking', async () => {
    const { user, api, section } = await openAutomation({ settings: { ...MOCK_SETTINGS, automation: 'full' } });
    await user.click(await section.findByRole('button', { name: 'Reset secret' }));
    const dialog = within(screen.getByRole('dialog', { name: 'Reset the secret?' }));
    expect(dialog.getByText(/Programs using the old one stop working/)).toBeInTheDocument();
    await user.click(dialog.getByRole('button', { name: 'Cancel' }));
    expect(api.tokenResets).toBe(0);

    await user.click(section.getByRole('button', { name: 'Reset secret' }));
    await user.click(within(screen.getByRole('dialog', { name: 'Reset the secret?' })).getByRole('button', { name: 'Reset secret' }));
    expect(await section.findByText(/New secret made/)).toBeInTheDocument();
    expect(api.tokenResets).toBe(1);
  });

  it('links to the guide', async () => {
    const { user, api, section } = await openAutomation();
    await user.click(section.getByRole('button', { name: 'How to connect the command line and AI tools' }));
    expect(api.opened).toEqual([AUTOMATION_DOCS]);
    expect(AUTOMATION_DOCS).toBe('https://github.com/mockydevs/mocky-mail/blob/main/docs/automation.md');
  });

  it('is not shown on phones', async () => {
    vi.stubGlobal(
      'matchMedia',
      vi.fn((query: string) => ({
        matches: query.startsWith('(max-width'),
        media: query,
        addEventListener: vi.fn(),
        removeEventListener: vi.fn(),
      })),
    );
    try {
      const { user, api } = renderApp({ phone: true });
      const info = vi.spyOn(api, 'getAutomationInfo');
      await rows();
      await user.click(screen.getByRole('button', { name: 'Folders and accounts' }));
      await user.click(screen.getByRole('button', { name: 'Settings' }));
      const settings = within(await screen.findByRole('region', { name: 'Settings' }));
      expect(settings.getByRole('region', { name: 'Accounts' })).toBeInTheDocument();
      expect(settings.queryByRole('region', { name: 'Automation' })).not.toBeInTheDocument();
      expect(info).not.toHaveBeenCalled();
    } finally {
      vi.unstubAllGlobals();
    }
  });
});

