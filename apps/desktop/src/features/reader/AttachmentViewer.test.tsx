import { vi } from 'vitest';
import { screen, within } from '@testing-library/react';

import { MOCK_MESSAGES } from '../../api/mock/mockData';
import { reader, renderApp, rows } from '../../test/renderApp';

// pdf.js needs a real canvas; jsdom has none. The PDF path is checked on devices.
vi.mock('./PdfPages', () => ({
  PdfPages: ({ fileName }: { fileName: string }) => <p>Pages of {fileName}</p>,
}));

const MB = 1024 * 1024;

/** The quotation email with a zip and a text file added. */
const messages = MOCK_MESSAGES.map((m) =>
  m.attachments.some((a) => a.fileName === 'Quotation-Westlands-rev2.pdf')
    ? {
        ...m,
        attachments: [
          ...m.attachments,
          { index: 2, fileName: 'Logo-final.zip', mimeType: 'application/zip', size: 6 * MB, preview: null },
          { index: 3, fileName: 'notes.txt', mimeType: 'text/plain', size: 40, preview: 'text' as const },
        ],
      }
    : m,
);

const viewer = () => screen.getByRole('dialog', { name: /^Preview of/ });

describe('attachment preview', () => {
  beforeEach(() => {
    // jsdom has no blob URLs.
    URL.createObjectURL = vi.fn(() => 'blob:preview');
    URL.revokeObjectURL = vi.fn();
  });

  it('opens a photo full screen on tap, and closes with Esc', async () => {
    const { user } = renderApp({ messages });
    await rows();
    await user.click(await within(reader()).findByRole('button', { name: 'Preview Floor-plan.png' }));
    expect(await within(viewer()).findByRole('img', { name: 'Floor-plan.png' })).toHaveAttribute('src', 'blob:preview');
    expect(within(viewer()).getByRole('button', { name: 'Close preview' })).toHaveFocus();
    await user.keyboard('{Escape}');
    await vi.waitFor(() => {
      expect(screen.queryByRole('dialog', { name: /^Preview of/ })).not.toBeInTheDocument();
    });
  });

  it("closes on the phone's Back button, leaving the message open", async () => {
    const { user } = renderApp({ messages });
    await rows();
    await user.click(await within(reader()).findByRole('button', { name: 'Preview Floor-plan.png' }));
    await within(viewer()).findByRole('img', { name: 'Floor-plan.png' });
    window.history.back();
    await vi.waitFor(() => {
      expect(screen.queryByRole('dialog', { name: /^Preview of/ })).not.toBeInTheDocument();
    });
    expect(within(reader()).getByRole('button', { name: 'Preview Floor-plan.png' })).toBeInTheDocument();
  });

  it('shows PDFs and text inside the app', async () => {
    const { user } = renderApp({ messages });
    await rows();
    await user.click(await within(reader()).findByRole('button', { name: 'Preview Quotation-Westlands-rev2.pdf' }));
    expect(await within(viewer()).findByText('Pages of Quotation-Westlands-rev2.pdf')).toBeInTheDocument();
    await user.click(within(viewer()).getByRole('button', { name: 'Close preview' }));
    await vi.waitFor(() => {
      expect(screen.queryByRole('dialog', { name: /^Preview of/ })).not.toBeInTheDocument();
    });

    await user.click(within(reader()).getByRole('button', { name: 'Preview notes.txt' }));
    expect(await within(viewer()).findByText(/Sample attachment text\./)).toBeInTheDocument();
  });

  it('saves and opens in another app from the preview', async () => {
    const { user, api } = renderApp({ messages });
    await rows();
    await user.click(await within(reader()).findByRole('button', { name: 'Preview Floor-plan.png' }));
    await user.click(within(viewer()).getByRole('button', { name: 'Save Floor-plan.png' }));
    expect(await within(viewer()).findByText('Saved: /home/you/Downloads/Floor-plan.png')).toBeInTheDocument();
    await user.click(within(viewer()).getByRole('button', { name: 'Open Floor-plan.png in another app' }));
    expect(api.openedFiles).toEqual(['Floor-plan.png']);
  });

  it('sends files it cannot show straight to another app', async () => {
    const { user, api } = renderApp({ messages });
    await rows();
    await user.click(await within(reader()).findByRole('button', { name: 'Open Logo-final.zip in another app' }));
    expect(api.openedFiles).toEqual(['Logo-final.zip']);
    expect(screen.queryByRole('dialog', { name: /^Preview of/ })).not.toBeInTheDocument();

    api.failNext('openAttachment', { kind: 'invalid', message: 'No app on this phone can open this file. Save it instead.' });
    await user.click(within(reader()).getByRole('button', { name: 'Open Logo-final.zip in another app' }));
    expect(await within(reader()).findByRole('alert')).toHaveTextContent('No app on this phone can open this file.');
  });

  it('offers Open and Save when a file turns out not to be what it says', async () => {
    const { user, api } = renderApp({ messages });
    await rows();
    api.failNext('previewAttachment', {
      kind: 'invalid',
      message: "Floor-plan.png can't be shown here. Open it in another app or save it.",
    });
    await user.click(await within(reader()).findByRole('button', { name: 'Preview Floor-plan.png' }));
    expect(await within(viewer()).findByRole('alert')).toHaveTextContent("Floor-plan.png can't be shown here.");
    await user.click(within(viewer()).getByRole('button', { name: 'Open in another app' }));
    expect(api.openedFiles).toEqual(['Floor-plan.png']);
  });

  it('keeps mail shortcuts from acting behind the preview', async () => {
    const { user } = renderApp({ messages });
    await rows();
    await user.click(await within(reader()).findByRole('button', { name: 'Preview Floor-plan.png' }));
    await within(viewer()).findByRole('img', { name: 'Floor-plan.png' });
    await user.keyboard('r');
    expect(screen.queryByRole('form', { name: 'Reply' })).not.toBeInTheDocument();
    expect(viewer()).toBeInTheDocument();
  });
});
