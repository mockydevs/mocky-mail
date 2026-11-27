import { appendTerm, applySuggestion, operatorAt, removeTerm, termLabel, tokenize } from './query';
import type { SearchTerm } from '../../api/types';

describe('search query helpers', () => {
  it('splits words, phrases, operators and negations', () => {
    const tokens = tokenize('from:boss  "site visit" -is:read subject:"big deal" https://x.example/a');
    expect(tokens.map((t) => [t.field, t.value, t.negated])).toEqual([
      ['from', 'boss', false],
      [null, 'site visit', false],
      ['is', 'read', true],
      ['subject', 'big deal', false],
      // Not an operator: a link stays plain text.
      [null, 'https://x.example/a', false],
    ]);
  });

  it('removes one term, found by what it says or by its place', () => {
    const t = (field: SearchTerm['field'], value: string, negated = false): SearchTerm => ({ field, value, negated });
    const phrase = t('text', 'site visit');
    const notRead = t('is', 'read', true);
    const terms = [t('from', 'boss'), phrase, notRead];
    const query = 'from:boss "site visit" -is:read';
    expect(removeTerm(query, terms, phrase)).toBe('from:boss -is:read');
    expect(removeTerm(query, terms, notRead)).toBe('from:boss "site visit"');
    // The backend tidied the value (today → a date): the only before: term still goes.
    // Taking one side of an OR takes the OR too.
    const amina = { ...t('from', 'amina'), orWithNext: true };
    const peter = t('from', 'peter');
    expect(removeTerm('invoice from:amina OR from:peter', [t('text', 'invoice'), amina, peter], peter)).toBe('invoice from:amina');
    expect(removeTerm('from:amina OR from:peter', [amina, peter], amina)).toBe('from:peter');
    const before = t('before', '2026-11-27');
    expect(removeTerm('invoice before:today', [before], before)).toBe('invoice');
  });

  it('knows which operator the caret is in, and puts a suggestion in', () => {
    const query = 'invoice from:wan is:';
    const context = operatorAt(query, 'invoice from:wan'.length);
    expect(context).toMatchObject({ field: 'from', partial: 'wan' });
    if (!context) return;
    expect(applySuggestion(query, context, 'wanjiru@fitout.example')).toEqual({
      query: 'invoice from:wanjiru@fitout.example is:',
      caret: 'invoice from:wanjiru@fitout.example '.length,
    });
    const atEnd = operatorAt(query, query.length);
    expect(atEnd).toMatchObject({ field: 'is', partial: '' });
    expect(operatorAt('invoice', 3)).toBeNull();
    // Folder names with spaces are quoted.
    const inContext = operatorAt('in:', 3);
    if (!inContext) throw new Error('expected an operator');
    expect(applySuggestion('in:', inContext, 'Old mail').query).toBe('in:"Old mail" ');
  });

  it('labels chips and appends examples', () => {
    expect(termLabel({ field: 'from', value: 'boss', negated: false })).toBe('from: boss');
    expect(termLabel({ field: 'is', value: 'read', negated: true })).toBe('not is: read');
    expect(termLabel({ field: 'text', value: 'site visit', negated: false })).toBe('“site visit”');
    expect(appendTerm('invoice ', 'has:attachment')).toBe('invoice has:attachment ');
    expect(appendTerm('', 'is:unread')).toBe('is:unread ');
  });
});
