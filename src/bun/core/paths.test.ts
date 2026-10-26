import { expect, test } from 'bun:test';
import { appDirs } from './paths';

test('macOS keeps config and data together under Application Support', () => {
  expect(appDirs({}, 'darwin', '/Users/u')).toEqual({
    config: '/Users/u/Library/Application Support/app.hearth.irc',
    data: '/Users/u/Library/Application Support/app.hearth.irc',
  });
});

test('Linux follows the XDG directories', () => {
  expect(appDirs({}, 'linux', '/home/u')).toEqual({ config: '/home/u/.config/app.hearth.irc', data: '/home/u/.local/share/app.hearth.irc' });
  expect(appDirs({ XDG_CONFIG_HOME: '/c', XDG_DATA_HOME: '/d' }, 'linux', '/home/u')).toEqual({ config: '/c/app.hearth.irc', data: '/d/app.hearth.irc' });
});

test('the override variables win', () => {
  expect(appDirs({ HEARTH_CONFIG_DIR: '/tmp/c', HEARTH_DATA_DIR: '/tmp/d' }, 'linux', '/home/u')).toEqual({ config: '/tmp/c', data: '/tmp/d' });
});
