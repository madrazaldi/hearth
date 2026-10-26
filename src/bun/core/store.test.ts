import { describe, expect, test } from 'bun:test';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { parse, param, tag, type Message } from './message';
import { Db, stripFormatting, type SavedBuffer, type SearchQuery } from './store';
import { parseMs } from './time';

function privmsg(time: string, nick: string, target: string, text: string, msgid?: string): Message {
  let raw = `@time=${time}`;
  if (msgid) raw += `;msgid=${msgid}`;
  raw += ` :${nick}!u@h PRIVMSG ${target} :${text}`;
  return parse(raw)!;
}

const log = (db: Db, target: string, msg: Message) => db.log({ network: 'srv', target, msg });
const text = (t: string, extra: Partial<SearchQuery> = {}): SearchQuery => ({ text: t, limit: 10, ...extra });

describe('store', () => {
  test('logs_pages_and_searches', () => {
    const db = Db.memory();
    log(db, '#a', privmsg('2026-01-01T00:00:01.000Z', 'ann', '#a', 'hello \x02world\x02', 'm1'));
    log(db, '#a', privmsg('2026-01-01T00:00:02.000Z', 'bob', '#a', '\x01ACTION waves at everyone\x01', 'm2'));
    log(db, '#a', privmsg('2026-01-01T00:00:02.000Z', 'bob', '#a', 'duplicate id', 'm2'));
    log(db, '#b', privmsg('2026-01-01T00:00:03.000Z', 'cat', '#b', 'worldly things'));
    db.flush();

    const all = db.history('srv', '#a', null, 10);
    expect(all.length).toBe(2); // duplicate msgid is ignored
    expect(tag(all[0], 'msgid')).toBe('m1');

    expect(db.history('srv', '#a', parseMs('2026-01-01T00:00:02.000Z'), 10).length).toBe(1);
    expect(db.historyAfter('srv', '#a', parseMs('2026-01-01T00:00:01.000Z')!, 10).length).toBe(1);

    const hits = db.search(text('worl'));
    expect(hits.length).toBe(2); // prefix match across buffers, formatting stripped
    expect(hits[0].target).toBe('#b'); // newest first
    expect(db.search(text('waves', { network: 'srv', target: '#a' })).length).toBe(1);
    expect(db.search(text('   '))).toEqual([]);
    expect(() => db.search(text('"unbalanced'))).not.toThrow();

    db.redact('srv', 'm1');
    db.flush();
    expect(db.history('srv', '#a', null, 10).length).toBe(1);
    expect(db.search(text('hello'))).toEqual([]);
  });

  test('reads see writes that are still queued', () => {
    const db = Db.memory();
    log(db, '#a', privmsg('2026-01-01T00:00:01.000Z', 'ann', '#a', 'queued'));
    expect(db.history('srv', '#a', null, 10).length).toBe(1);
  });

  test('search_filters', () => {
    const db = Db.memory();
    log(db, '#a', privmsg('2026-01-01T00:00:01.000Z', 'Ann', '#a', 'deploy one'));
    log(db, '#a', privmsg('2026-01-01T00:00:02.000Z', 'bob', '#a', 'deploy two'));
    log(db, '#b', privmsg('2026-01-01T00:00:03.000Z', 'ann', '#b', 'deploy three'));
    log(db, '#b', privmsg('2026-01-01T00:00:04.000Z', 'ann', '#b', 'unrelated'));
    db.flush();
    const texts = (q: SearchQuery) => db.search(q).map((h) => param(h.msg, 1));

    expect(texts(text('deploy', { from: 'ANN' }))).toEqual(['deploy three', 'deploy one']);
    const t2 = parseMs('2026-01-01T00:00:02.000Z')!;
    expect(texts(text('deploy', { before: t2 }))).toEqual(['deploy one']);
    expect(texts(text('deploy', { after: t2 }))).toEqual(['deploy three']);
    // Paging: the next page starts before the oldest hit so far.
    expect(texts(text('deploy', { limit: 2 }))).toEqual(['deploy three', 'deploy two']);
    expect(texts(text('deploy', { limit: 2, before: t2 }))).toEqual(['deploy one']);
    // Filters alone list everything they match; no filters at all, nothing.
    expect(texts(text('', { from: 'ann', target: '#b' }))).toEqual(['unrelated', 'deploy three']);
    expect(texts(text(''))).toEqual([]);
  });

  test('bouncer child networks are found by their server', () => {
    const db = Db.memory();
    db.log({ network: 'soju/1', target: '#a', msg: privmsg('2026-01-01T00:00:01.000Z', 'ann', '#a', 'needle') });
    db.log({ network: 'other', target: '#a', msg: privmsg('2026-01-01T00:00:02.000Z', 'ann', '#a', 'needle') });
    db.flush();
    expect(db.search(text('needle', { network: 'soju' })).map((h) => h.network)).toEqual(['soju/1']);
    db.forgetServer('soju');
    expect(db.search(text('needle')).map((h) => h.network)).toEqual(['other']);
  });

  test('renamed_channels_keep_their_log', () => {
    const db = Db.memory();
    log(db, '#old', privmsg('2026-01-01T00:00:01.000Z', 'ann', '#old', 'before'));
    db.renameTarget('srv', '#old', '#new');
    log(db, '#new', privmsg('2026-01-01T00:00:02.000Z', 'ann', '#new', 'after'));
    db.flush();
    expect(db.history('srv', '#old', null, 10)).toEqual([]);
    expect(db.history('srv', '#new', null, 10).map((m) => param(m, 1))).toEqual(['before', 'after']);
  });

  test('logging_can_be_disabled', () => {
    const db = Db.memory();
    db.setLogging(false);
    log(db, '#a', privmsg('2026-01-01T00:00:01.000Z', 'ann', '#a', 'secret'));
    db.setLogging(true);
    db.flush();
    expect(db.history('srv', '#a', null, 10)).toEqual([]);
  });

  test('saves_buffers_and_sts', () => {
    const db = Db.memory();
    const bufs: SavedBuffer[] = [
      { name: '#Rust', kind: 'channel', key: 'k' },
      { name: 'Ann', kind: 'query', key: '' },
    ];
    db.saveBuffers('srv', bufs, (s) => s.toLowerCase());
    const back = db.buffers('srv');
    expect(back.length).toBe(2);
    expect(back[0].name).toBe('#Rust');
    expect(back[0].key).toBe('k');
    // Saving again replaces the list.
    db.saveBuffers('srv', [bufs[1]], (s) => s.toLowerCase());
    expect(db.buffers('srv').length).toBe(1);

    expect(db.stsPolicy('irc.example')).toBeNull();
    db.setStsPolicy('IRC.example', 6697, 3600);
    expect(db.stsPolicy('irc.example')?.port).toBe(6697);
    db.setStsPolicy('irc.example', 6697, 0);
    expect(db.stsPolicy('irc.example')).toBeNull();
  });

  test('strips_formatting', () => {
    expect(stripFormatting('\x0304,12red\x03 \x02b\x02 \x04ff0000,00ff00hex')).toBe('red b hex');
    expect(stripFormatting('\x03,5')).toBe(',5');
  });

  test('stamps_messages_without_a_time_tag_and_keeps_the_stored_shape', () => {
    const db = Db.memory();
    const before = Date.now();
    const msg = parse(':ann!u@h PRIVMSG #a :no time tag')!;
    log(db, '#a', msg);
    const [back] = db.history('srv', '#a', null, 10);
    expect(msg.tags.time).toBeUndefined(); // the caller's message is untouched
    expect(parseMs(back.tags.time)!).toBeGreaterThanOrEqual(before);
    expect(back.source).toEqual({ nick: 'ann', user: 'u', host: 'h' });
  });

  test('clearing the log leaves search working for what comes after', () => {
    const db = Db.memory();
    for (let i = 0; i < 1000; i++) log(db, '#a', privmsg('2026-01-01T00:00:01.000Z', 'ann', '#a', `old ${i}`));
    db.clearLogs();
    expect(db.search(text('old'))).toEqual([]);
    log(db, '#a', privmsg('2026-01-01T00:00:02.000Z', 'ann', '#a', 'fresh line'));
    expect(db.search(text('fresh')).length).toBe(1);
    // The delete trigger is back, so the index follows deletions again.
    db.forgetServer('srv');
    expect(db.search(text('fresh'))).toEqual([]);
  });

  test('opens_and_reopens_a_file_database', () => {
    const dir = mkdtempSync(join(tmpdir(), 'hearth-db-'));
    try {
      const path = join(dir, 'nested', 'hearth.db');
      const db = Db.open(path);
      log(db, '#a', privmsg('2026-01-01T00:00:01.000Z', 'ann', '#a', 'persisted'));
      db.setStsPolicy('irc.example', 6697, 60);
      db.close();
      const again = Db.open(path);
      expect(again.history('srv', '#a', null, 10).length).toBe(1);
      expect(again.search(text('persist')).length).toBe(1);
      expect(again.stsPolicy('irc.example')?.port).toBe(6697);
      again.clearLogs();
      expect(again.history('srv', '#a', null, 10)).toEqual([]);
      expect(again.search(text('persist'))).toEqual([]);
      again.close();
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });
});
