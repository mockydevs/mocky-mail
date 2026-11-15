import { afterEach } from 'vitest';

import { contrast, fixContrast, readableOn } from './contrast';

afterEach(() => {
  document.body.innerHTML = '';
  document.documentElement.removeAttribute('style');
});

const WHITE = { r: 255, g: 255, b: 255 };
const BLACK = { r: 0, g: 0, b: 0 };

describe('contrast', () => {
  it('matches the WCAG ratios', () => {
    expect(contrast(WHITE, BLACK)).toBeCloseTo(21, 0);
    expect(contrast(WHITE, WHITE)).toBeCloseTo(1, 5);
  });

  it('darkens pale text on a light background, keeping its hue', () => {
    const grey = { r: 170, g: 170, b: 170 };
    const fixed = readableOn(grey, WHITE, 4.5);
    expect(contrast(fixed, WHITE)).toBeGreaterThanOrEqual(4.5);
    expect(fixed.r).toBe(fixed.g);
  });

  it('lightens dark text on a dark background', () => {
    const navy = { r: 19, g: 84, b: 122 };
    const fixed = readableOn({ r: 43, g: 43, b: 40 }, navy, 4.5);
    expect(contrast(fixed, navy)).toBeGreaterThanOrEqual(4.5);
  });
});

describe('fixContrast', () => {
  it('rescues white text whose background picture was removed', () => {
    document.documentElement.style.backgroundColor = 'rgb(255, 255, 255)';
    document.body.innerHTML = '<p id="hidden" style="color:#ffffff">To a life well-lived</p><p id="fine" style="color:#222222">Hello</p>';
    fixContrast(document);
    const hidden = document.getElementById('hidden');
    expect(hidden?.style.getPropertyPriority('color')).toBe('important');
    // Readable text is left exactly as the sender styled it.
    expect(document.getElementById('fine')?.style.color).toBe('rgb(34, 34, 34)');
    expect(document.getElementById('fine')?.style.getPropertyPriority('color')).toBe('');
  });

  it('checks text against the box behind it, not the page', () => {
    document.documentElement.style.backgroundColor = 'rgb(255, 255, 255)';
    document.body.innerHTML = '<div style="background-color:#13547a"><p id="title" style="color:#ffffff">Live Rating Notice</p></div>';
    fixContrast(document);
    // White on dark blue already reads: unchanged.
    expect(document.getElementById('title')?.style.getPropertyPriority('color')).toBe('');
  });
});
