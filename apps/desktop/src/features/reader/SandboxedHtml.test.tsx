import { render, screen } from '@testing-library/react';

import { EMAIL_CSP, emailDocument, SandboxedHtml } from './SandboxedHtml';

describe('SandboxedHtml', () => {
  it('renders in an iframe with an empty sandbox and the CSP', () => {
    render(<SandboxedHtml title="Message body" html="<p>Hello</p>" />);
    const frame = screen.getByTitle('Message body');
    expect(frame.tagName).toBe('IFRAME');
    // Only popups are allowed (links open via the backend's new-window guard);
    // never scripts, same-origin, forms or top navigation.
    expect(frame.getAttribute('sandbox')).toBe('allow-popups');
    expect(frame.getAttribute('referrerpolicy')).toBe('no-referrer');
    const doc = frame.getAttribute('srcdoc') ?? '';
    expect(doc).toContain(`<meta http-equiv="Content-Security-Policy" content="${EMAIL_CSP}">`);
    expect(doc).toContain('<p>Hello</p>');
  });

  it('grows to fill a flex container instead of using a fixed size', () => {
    render(
      <div data-testid="pane" style={{ display: 'flex', flexDirection: 'column' }}>
        <SandboxedHtml title="Body" html="<p>x</p>" />
      </div>,
    );
    const frame = screen.getByTitle('Body');
    const parent = frame.parentElement;
    expect(parent).toHaveStyle({ display: 'flex', flexDirection: 'column' });
    expect(frame.style.flexGrow).toBe('1');
    expect(frame.style.minHeight).toBe('0px');
    expect(frame.style.height).toBe('');
    expect(frame).not.toHaveAttribute('height');
    expect(frame.style.width).toBe('100%');
  });

  it('renders the email on a white, light-scheme page', () => {
    const doc = emailDocument('<p>x</p>');
    expect(doc).toContain('<meta name="color-scheme" content="light">');
    expect(doc).toContain(':root{color-scheme:light}');
    expect(doc).toContain('background:#fff');
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
