import { svelte } from '@sveltejs/vite-plugin-svelte';
import tailwindcss from '@tailwindcss/vite';
import { fileURLToPath } from 'node:url';
import { defineConfig, type Plugin } from 'vite';
import { electrobunAliases } from './devkit.ts';

// The packaged UI runs under this policy; the dev server and preview don't, so hot reload keeps working.
// `ws://127.0.0.1:*` is Electrobun's local socket to the Bun process.
const csp = [
  "default-src 'self' views:",
  "script-src 'self' views:",
  "style-src 'self' views: 'unsafe-inline'",
  "img-src 'self' views: data: blob:",
  "font-src 'self' views: data:",
  'connect-src ws://127.0.0.1:*',
  "object-src 'none'",
  "base-uri 'none'",
  "frame-src 'none'",
  "form-action 'none'",
].join('; ');

const contentSecurityPolicy = (): Plugin => ({
  name: 'hearth:csp',
  apply: 'build',
  transformIndexHtml: () => [{ tag: 'meta', attrs: { 'http-equiv': 'Content-Security-Policy', content: csp }, injectTo: 'head-prepend' }],
});

export default defineConfig(async () => ({
  plugins: [tailwindcss(), svelte(), contentSecurityPolicy()],
  resolve: {
    alias: [{ find: '$lib', replacement: fileURLToPath(new URL('./src/lib', import.meta.url)) }, ...(await electrobunAliases())],
  },
  clearScreen: false,
  // `bun run dev` is the browser preview (src/lib/mock.ts); the desktop app serves the built files.
  server: { port: 1420, strictPort: true },
  // The app ships its own Chromium (CEF), so a modern target is enough.
  build: { target: 'chrome120', chunkSizeWarningLimit: 2000 },
}));
