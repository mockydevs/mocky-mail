import { render, screen } from '@testing-library/react';

import { EMAIL_CSP, emailDocument, SandboxedHtml } from './SandboxedHtml';

describe('SandboxedHtml', () => {
  it('renders in an iframe with an empty sandbox and the CSP', () => {
    render(<SandboxedHtml title="Message body" html="<p>Hello</p>" />);
    const frame = screen.getByTitle('Message body');
    expect(frame.tagName).toBe('IFRAME');
    expect(frame.getAttribute('sandbox')).toBe('');
    expect(frame.getAttribute('referrerpolicy')).toBe('no-referrer');
    const doc = frame.getAttribute('srcdoc') ?? '';
    expect(doc).toContain(`<meta http-equiv="Content-Security-Policy" content="${EMAIL_CSP}">`);
    expect(doc).toContain('<p>Hello</p>');
  });

  it('puts the CSP before any message content', () => {
    const doc = emailDocument('<p>x</p>');
    expect(doc.indexOf('Content-Security-Policy')).toBeLessThan(doc.indexOf('<p>x</p>'));
  });

  it('allows no scripts, frames or connections', () => {
    expect(EMAIL_CSP).toContain("default-src 'none'");
    expect(EMAIL_CSP).not.toMatch(/script-src|unsafe-eval|frame-src|connect-src/);
  });
});
