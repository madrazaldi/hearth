import { svelte } from '@sveltejs/vite-plugin-svelte';
import { defineConfig } from 'vitest/config';

export default defineConfig({
  plugins: [svelte()],
  // Resolve Svelte's client build so runes are reactive in tests.
  resolve: { conditions: ['browser'] },
  test: { environment: 'jsdom', include: ['src/**/*.test.ts'] },
});
