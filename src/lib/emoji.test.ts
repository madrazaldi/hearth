import { describe, expect, it } from 'vitest';
import { emojiByName, emojiQuery, searchEmoji, typedShortcode } from './emoji';

describe('emoji', () => {
  it('finds the query being typed', () => {
    expect(emojiQuery(':j')).toEqual({ start: 0, query: 'j' });
    expect(emojiQuery('lol :Jo')).toEqual({ start: 4, query: 'jo' });
    expect(emojiQuery('nice :+1')).toEqual({ start: 5, query: '+1' });
  });

  it('ignores colons that do not start a word', () => {
    expect(emojiQuery('at 12:30')).toBeNull();
    expect(emojiQuery('https://x')).toBeNull();
    expect(emojiQuery('nick:')).toBeNull();
    expect(emojiQuery(':joy: ')).toBeNull();
    expect(emojiQuery(':')).toBeNull();
  });

  it('ranks prefix matches first, in gemoji order', () => {
    const names = searchEmoji('jo').map((e) => e.name);
    expect(names[0]).toBe('joy');
    expect(searchEmoji('j', 50).map((e) => e.name)).toContain('joy');
    expect(searchEmoji('j').length).toBe(7);
  });

  it('falls back to names containing the query', () => {
    const names = searchEmoji('eyes').map((e) => e.name);
    expect(names[0]).toBe('eyes');
    expect(names).toContain('heart_eyes');
    expect(searchEmoji('zzzqqq')).toEqual([]);
  });

  it('expands a complete shortcode', () => {
    expect(emojiByName('joy')).toBe('😂');
    expect(typedShortcode('haha :joy:')).toEqual({ start: 5, char: '😂' });
    expect(typedShortcode(':THUMBSUP:')).toEqual({ start: 0, char: '👍' });
    expect(typedShortcode('haha :notanemoji:')).toBeNull();
    expect(typedShortcode('a:joy:')).toBeNull();
  });
});
