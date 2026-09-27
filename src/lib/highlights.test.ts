// Per-network highlight words and "never highlight" senders, on top of the global list.

import { beforeEach, describe, expect, it, vi } from 'vitest';
import { connect, feed, mockApi, msg, sent } from './test-utils';

vi.mock('./api', () => ({ api: mockApi, onIrc: vi.fn(async () => () => {}) }));
vi.mock('./notify', () => ({ notify: vi.fn() }));

const { App } = await import('./state.svelte');
const { historyLine } = await import('./incoming');
const { notify } = await import('./notify');

let app: InstanceType<typeof App>;

beforeEach(() => {
  vi.clearAllMocks();
  sent.length = 0;
  app = new App();
  app.focused = false;
  connect(app);
  feed(app, ':me!u@h JOIN #a');
});

const conn = () => app.conns.get('c')!;
const highlights = () => conn().buffer('#a')!.messages.map((l) => [l.text, l.highlight]);

describe('network highlight rules', () => {
  it('adds words and regexes for this server to the nick and global words', () => {
    app.updateSettings({
      highlightWords: ['hearth'],
      networkHighlights: { c: { words: ['deploy', '/\\bci (failed|broke)\\b/i'], never: [] } },
    });
    feed(
      app,
      ':ann!a@h PRIVMSG #a :me: ping',
      ':ann!a@h PRIVMSG #a :hearth is nice',
      ':ann!a@h PRIVMSG #a :deploy done',
      ':ann!a@h PRIVMSG #a :CI broke again',
      ':ann!a@h PRIVMSG #a :redeploying',
    );
    expect(highlights()).toEqual([
      ['me: ping', true],
      ['hearth is nice', true],
      ['deploy done', true],
      ['CI broke again', true],
      ['redeploying', false],
    ]);
    expect(notify).toHaveBeenCalledTimes(4);
  });

  it("doesn't apply another server's rules", () => {
    app.updateSettings({ networkHighlights: { other: { words: ['deploy'], never: [] } } });
    feed(app, ':ann!a@h PRIVMSG #a :deploy done');
    expect(highlights()).toEqual([['deploy done', false]]);
  });

  it('never highlights listed nicks or masks, even for our nick', () => {
    app.updateSettings({ networkHighlights: { c: { words: [], never: ['GitBot', '*!*@relay.example'] } } });
    feed(
      app,
      ':gitbot!g@h PRIVMSG #a :me pushed 3 commits',
      ':bridge!b@relay.example PRIVMSG #a :<x> me: hi',
      ':ann!a@h PRIVMSG #a :me: hi',
    );
    expect(highlights()).toEqual([
      ['me pushed 3 commits', false],
      ['<x> me: hi', false],
      ['me: hi', true],
    ]);
    expect(notify).toHaveBeenCalledTimes(1);
  });

  it('applies to replayed history too', () => {
    app.updateSettings({ networkHighlights: { c: { words: ['deploy'], never: ['gitbot'] } } });
    const line = (raw: string) => historyLine(app, conn(), msg(raw))!.highlight;
    expect(line(':ann!a@h PRIVMSG #a :deploy now')).toBe(true);
    expect(line(':GitBot!g@h PRIVMSG #a :deploy now')).toBe(false);
  });
});
