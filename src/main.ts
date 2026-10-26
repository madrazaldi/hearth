import '@fontsource-variable/geist';
import '@fontsource-variable/geist-mono';
import './app.css';
import { mount } from 'svelte';

// Outside the desktop app (`bun run dev`) there is no backend: script one.
if (import.meta.env.DEV && window.__electrobunWebviewId === undefined) await import('./lib/mock');
import App from './App.svelte';

export default mount(App, { target: document.getElementById('app')! });

