// Keyboard shortcuts shown in the help overlay and Settings. Keep in sync with
// the handlers in App.svelte, BufferView.svelte and Composer.svelte, and README.

const isMac = typeof navigator !== 'undefined' && navigator.userAgent.includes('Mac');
export const MOD = isMac ? '⌘' : 'Ctrl';
const ALT = isMac ? '⌥' : 'Alt';

export interface Shortcut {
  /** Alternative key combos, each a list of keys pressed together. */
  keys: string[][];
  label: string;
}

export const SHORTCUT_GROUPS: { title: string; items: Shortcut[] }[] = [
  {
    title: 'Navigation',
    items: [
      { keys: [[MOD, 'K']], label: 'Jump to a buffer' },
      { keys: [[ALT, '↑'], [ALT, '↓']], label: 'Previous / next buffer' },
      { keys: [[ALT, '⇧', '↑'], [ALT, '⇧', '↓']], label: 'Previous / next unread' },
      { keys: [[MOD, '1–9']], label: 'Go to channel by position' },
      { keys: [[MOD, 'W']], label: 'Close buffer' },
    ],
  },
  {
    title: 'Panels',
    items: [
      { keys: [[MOD, 'F']], label: 'Filter messages' },
      { keys: [[MOD, '⇧', 'F']], label: 'Search all messages' },
      { keys: [[MOD, '⇧', 'M']], label: 'Toggle member list' },
      { keys: [[MOD, ',']], label: 'Settings' },
      { keys: [[MOD, '/']], label: 'Keyboard shortcuts' },
      { keys: [['Esc']], label: 'Close panel, cancel reply' },
    ],
  },
  {
    title: 'Composer',
    items: [
      { keys: [['Tab'], ['⇧', 'Tab']], label: 'Complete nick, channel, command or alias' },
      { keys: [['↑'], ['↓']], label: 'Input history' },
      { keys: [['⇧', '↵']], label: 'New line' },
    ],
  },
];
