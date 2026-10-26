import { svelte } from '@sveltejs/vite-plugin-svelte';
import { fileURLToPath } from 'node:url';
import { defineConfig } from 'vitest/config';
import { electrobunAliases } from './devkit.ts';

export default defineConfig(async () => ({
  plugins: [svelte()],
  // Resolve Svelte's client build so runes are reactive in tests.
  resolve: {
    conditions: ['browser'],
    alias: [{ find: '$lib', replacement: fileURLToPath(new URL('./src/lib', import.meta.url)) }, ...(await electrobunAliases())],
  },
  // The Bun-side tests run under `bun test` (src/bun).
  test: { environment: 'happy-dom', include: ['src/**/*.test.ts'], exclude: ['src/bun/**', 'node_modules/**'] },
}));
