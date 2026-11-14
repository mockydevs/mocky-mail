/// <reference types="node" />
import { readFileSync } from 'node:fs';
import { join } from 'node:path';

import { screen } from '@testing-library/react';

import { renderApp, rows } from '../test/renderApp';

// The real stylesheet (CSS is not processed in tests, so read the file).
const css = readFileSync(join(process.cwd(), 'src/app/app.css'), 'utf8');

/** Declarations of a top-level rule (outside @media) with exactly this selector. */
function rule(selector: string): Record<string, string[]> {
  const clean = css.replace(/\/\*[\s\S]*?\*\//g, '');
  const out: Record<string, string[]> = {};
  let depth = 0;
  let start = 0;
  let found = false;
  for (let i = 0; i < clean.length; i++) {
    const ch = clean[i];
    if (ch === '{') {
      if (depth === 0) {
        const head = clean.slice(clean.lastIndexOf('}', i) + 1, i).trim().replace(/\s+/g, ' ');
        if (head === selector) {
          found = true;
          start = i + 1;
        }
      }
      depth++;
    } else if (ch === '}') {
      depth--;
      if (depth === 0 && found) {
        for (const decl of clean.slice(start, i).split(';')) {
          const [prop, ...value] = decl.split(':');
          if (!prop?.trim()) continue;
          (out[prop.trim()] ??= []).push(value.join(':').trim());
        }
        return out;
      }
    }
  }
  throw new Error(`no rule for ${selector}`);
}

describe('shell layout', () => {
  it('pins the window: html, body and #root never scroll', () => {
    const root = rule('html, body, #root');
    expect(root['height']).toEqual(['100%']);
    expect(root['overflow']).toEqual(['hidden']);
    expect(root['margin']).toEqual(['0']);
  });

  it('makes the app exactly one viewport tall, with a dvh value after the vh fallback', () => {
    const app = rule('.app');
    expect(app['height']).toEqual(['100vh', '100dvh']);
    expect(app['overflow']).toEqual(['hidden']);
    expect(app['flex-direction']).toEqual(['column']);
    expect(rule('.topbar, .statusbar')['flex']).toEqual(['none']);
    const main = rule('.workspace');
    expect(main['flex']).toEqual(['1 1 auto']);
    expect(main['min-height']).toEqual(['0']);
    expect(main['overflow']).toEqual(['hidden']);
  });

  it.each(['.sidebar__scroll', '.list__scroll', '.reader__scroll', '.reader__head', '.settings__body', '.panel', '.compose'])(
    '%s is its own scroll container',
    (selector) => {
      const r = rule(selector);
      expect(r['overflow-y']).toEqual(['auto']);
      expect(r['min-height']).toEqual(['0']);
    },
  );

  it.each(['.sidebar', '.panes', '.list', '.reader'])('%s can shrink below its content (min-height 0)', (selector) => {
    expect(rule(selector)['min-height']).toEqual(['0']);
  });

  it('lets the HTML email fill the rest of the reading pane', () => {
    const frame = rule('.reader__frame');
    expect(frame['flex']).toEqual(['1 1 auto']);
    expect(frame['min-height']).toEqual(['0']);
    const iframe = rule('.email-frame');
    expect(iframe['height']).toBeUndefined();
    expect(iframe['max-width']).toBeUndefined();
    expect(iframe['background']).toEqual(['#fff']);
    expect(rule('.reader__text')['max-width']).toEqual(['80ch']);
  });

  it('renders top bar, main row and status bar in that order, with New message outside the folder scroller', async () => {
    renderApp();
    await rows();
    const app = document.querySelector('.app');
    expect([...(app?.children ?? [])].map((c) => c.className.split(' ')[0])).toEqual(['topbar', 'workspace', 'statusbar']);
    const scroller = document.querySelector('.sidebar__scroll');
    const newMessage = screen.getByRole('button', { name: 'New message' });
    expect(scroller?.contains(newMessage)).toBe(false);
    expect(scroller?.contains(screen.getByRole('list', { name: 'Work' }))).toBe(true);
  });
});
