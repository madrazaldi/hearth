// User aliases: expansion, chaining, shadowing built-ins and runaway guards.

import { beforeEach, describe, expect, it, vi } from 'vitest';
import { connect, feed, mockApi, sent } from './test-utils';
import { aliasError, expandCommand, splitCommands } from './aliases';

vi.mock('./api', () => ({ api: mockApi, onIrc: vi.fn(async () => () => {}) }));
vi.mock('./notify', () => ({ notify: vi.fn() }));

const { App } = await import('./state.svelte');
const { commandInfos, runInput } = await import('./commands');

let app: InstanceType<typeof App>;

beforeEach(() => {
  vi.clearAllMocks();
  sent.length = 0;
  app = new App();
  connect(app, { isupport: { NETWORK: 'Testnet' } });
  feed(app, ':me!u@h JOIN #a');
  sent.length = 0;
});

const buf = () => app.conns.get('c')!.buffer('#a')!;
const toasts = () => app.toasts.map((t) => t.text);
const run = (input: string) => runInput(app, buf(), input);
const aliases = (a: Record<string, string>) => app.updateSettings({ aliases: a });

describe('expansion', () => {
  const vars = { nick: 'me', channel: '#a', network: 'Testnet' };

  it('substitutes positional, rest and context variables', () => {
    expect(expandCommand('/msg $1 $2-', ['ann', 'hi', 'there'], vars)).toBe('/msg ann hi there');
    expect(expandCommand('/say $* in $channel on $network as $nick', ['x', 'y'], vars)).toBe('/say x y in #a on Testnet as me');
    expect(expandCommand('/kick $1 $3', ['ann'], vars)).toBe('/kick ann');
    expect(expandCommand('$nickname costs $5', [], vars)).toBe('$nickname costs');
  });

  it('splits on ; with \; as a literal', () => {
    expect(splitCommands(' /a ; /b x;; hi\\; there ')).toEqual(['/a', '/b x', 'hi; there']);
  });

  it('validates names and bodies', () => {
    expect(aliasError('ns', '/msg NickServ $*')).toBeNull();
    expect(aliasError('', '/x')).toMatch(/name/);
    expect(aliasError('n s', '/x')).toMatch(/letters/);
    expect(aliasError('ns', ' ; ')).toMatch(/command/);
  });
});

describe('running aliases', () => {
  it('runs each command with arguments filled in', async () => {
    aliases({ ns: '/msg NickServ $*', greet: 'hello $1!; /me waves at $1' });
    expect(await run('/ns identify hunter2')).toBe(true);
    expect(await run('/GREET ann')).toBe(true);
    expect(sent).toEqual(['PRIVMSG NickServ :identify hunter2', 'PRIVMSG #a :hello ann!', 'PRIVMSG #a :\x01ACTION waves at ann\x01']);
  });

  it('keeps ; inside arguments intact', async () => {
    aliases({ say: '/msg #a $*' });
    await run('/say a;b');
    expect(sent).toEqual(['PRIVMSG #a :a;b']);
  });

  it('can call other aliases', async () => {
    aliases({ j: '/join #$1', both: '/j one; /j two' });
    await run('/both');
    expect(sent).toEqual(['JOIN #one', 'JOIN #two']);
  });

  it('overrides a built-in but can still call it from inside', async () => {
    aliases({ join: '/join #$1; /msg ChanServ op #$1' });
    await run('/join rust');
    expect(sent).toEqual(['JOIN #rust', 'PRIVMSG ChanServ :op #rust']);
  });

  it('stops recursion and runaway fan-out', async () => {
    aliases({ a: '/b', b: '/a' });
    expect(await run('/a')).toBe(false);
    expect(toasts()).toContain('Alias /a calls itself');
    app.toasts = [];
    aliases({ x: '/y $1; /y $1; /y $1; /y $1', y: '/z $1; /z $1; /z $1; /z $1', z: '/raw PING $1; /raw PING $1; /raw PING $1; /raw PING $1' });
    await run('/x 1');
    expect(sent.length).toBeLessThanOrEqual(50);
    expect(toasts()).toContain('Alias /x runs too many commands');
    expect(sent.every((l) => l === 'PING 1')).toBe(true);
  });

  it('lists aliases ahead of the built-ins they replace', () => {
    const infos = commandInfos({ join: '/join #$1', ns: '/msg NickServ $*' });
    expect(infos.slice(0, 2).map((c) => c.usage)).toEqual(['/join', '/ns']);
    expect(infos.filter((c) => c.name === 'join')).toHaveLength(1);
    expect(infos.some((c) => c.name === 'part')).toBe(true);
  });
});
