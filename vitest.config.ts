import { svelte } from '@sveltejs/vite-plugin-svelte';
import { fileURLToPath } from 'node:url';
import { defineConfig } from 'vitest/config';

export default defineConfig({
  plugins: [svelte()],
  // Resolve Svelte's client build so runes are reactive in tests.
  resolve: {
    conditions: ['browser'],
    alias: { $lib: fileURLToPath(new URL('./src/lib', import.meta.url)) },
  },
  test: { environment: 'happy-dom', include: ['src/**/*.test.ts'] },
});
