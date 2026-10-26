// The quiet update check must not toast over the first-run welcome card.

import { beforeEach, describe, expect, it, vi } from 'vitest';
import { mockApi } from './test-utils';

vi.mock('./api', () => ({ api: mockApi, onIrc: vi.fn(async () => () => {}) }));
vi.mock('./notify', () => ({ notify: vi.fn() }));
vi.mock('./platform', () => ({
  platform: {
    version: vi.fn(async () => '0.1.0'),
    checkForUpdate: vi.fn(async () => ({ version: '0.2.0' })),
    installUpdate: vi.fn(),
    relaunch: vi.fn(),
  },
}));

const { app } = await import('./state.svelte');
const { updater } = await import('./updater.svelte');

beforeEach(() => {
  app.toasts = [];
  app.ready = true;
});

describe('update announcement', () => {
  it('waits until the welcome screen is gone', async () => {
    app.servers = [];
    await updater.check(true);
    expect(updater.phase).toBe('available');
    expect(app.toasts).toEqual([]);

    app.servers = [{ id: 's' } as (typeof app.servers)[number]];
    app.onboarding = true;
    updater.announce();
    expect(app.toasts).toEqual([]);

    app.onboarding = false;
    updater.announce();
    updater.announce();
    expect(app.toasts.map((t) => t.text)).toEqual(['Hearth 0.2.0 is available. Install it from Settings › General.']);
  });

  it('announces a version only once', async () => {
    await updater.check(true);
    updater.announce();
    expect(app.toasts).toEqual([]);
  });
});
