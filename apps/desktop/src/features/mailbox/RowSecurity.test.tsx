import { within } from '@testing-library/react';
import { afterEach, vi } from 'vitest';

import { MOCK_MESSAGES } from '../../api/mock/mockData';
import { MOCK_SECURITY_MESSAGES } from '../../api/mock/mockSecurity';
import { renderApp, rows } from '../../test/renderApp';

const messages = [...MOCK_MESSAGES, ...MOCK_SECURITY_MESSAGES];

async function row(subject: RegExp, options: Parameters<typeof renderApp>[0] = {}) {
  renderApp({ messages, ...options });
  return within((await rows()).getByRole('button', { name: subject }));
}

describe('signed and encrypted rows', () => {
  it('show a seal for signed mail and a lock for encrypted mail, with labels', async () => {
    renderApp({ messages });
    const list = await rows();
    const both = within(list.getByRole('button', { name: /Site access codes/ }));
    expect(both.getByLabelText('Signed')).toHaveClass('row__icon--seal');
    expect(both.getByLabelText('Encrypted')).toHaveClass('row__icon--lock');

    const signed = within(list.getByRole('button', { name: /Signed contract for the fit-out/ }));
    expect(signed.getByLabelText('Signed')).toBeInTheDocument();
    expect(signed.queryByLabelText('Encrypted')).not.toBeInTheDocument();

    // A signature that didn't check out is still marked as signed; the reader says what was wrong.
    expect(within(list.getByRole('button', { name: /New bank details/ })).getByLabelText('Signed')).toBeInTheDocument();

    const locked = within(list.getByRole('button', { name: /Encrypted message/ }));
    expect(locked.getByLabelText('Encrypted')).toBeInTheDocument();
    expect(locked.queryByLabelText('Signed')).not.toBeInTheDocument();

    const plain = within(list.getByRole('button', { name: /Revised quotation/ }));
    expect(plain.queryByLabelText('Signed')).not.toBeInTheDocument();
    expect(plain.queryByLabelText('Encrypted')).not.toBeInTheDocument();
  });

  it('name the icons in the row’s accessible name', async () => {
    renderApp({ messages });
    const list = await rows();
    expect(list.getByRole('button', { name: /Site access codes/ })).toHaveAccessibleName(/Signed.*Encrypted/);
  });

  describe('on a phone', () => {
    afterEach(() => {
      vi.unstubAllGlobals();
    });

    it('keep the icons', async () => {
      vi.stubGlobal(
        'matchMedia',
        vi.fn((query: string) => ({ matches: query.startsWith('(max-width'), media: query, addEventListener: vi.fn(), removeEventListener: vi.fn() })),
      );
      const site = await row(/Site access codes/, { phone: true });
      expect(site.getByLabelText('Signed')).toBeInTheDocument();
      expect(site.getByLabelText('Encrypted')).toBeInTheDocument();
    });
  });
});
