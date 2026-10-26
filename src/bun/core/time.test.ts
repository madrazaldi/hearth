import { expect, test } from 'bun:test';
import { formatMs, parseMs } from './time';

test('roundtrips_timestamps', () => {
  const t = parseMs('2026-09-27T12:34:56.789Z')!;
  expect(t).toBe(1_790_512_496_789);
  expect(formatMs(t)).toBe('2026-09-27T12:34:56.789Z');
  expect(parseMs('1970-01-01T00:00:00Z')).toBe(0);
  expect(parseMs('2024-02-29T00:00:00.5Z')).toBe(1_709_164_800_500);
  expect(parseMs('garbage')).toBeNull();
  expect(parseMs('2024-02-29T00:00:00+01:00')).toBeNull();
});
