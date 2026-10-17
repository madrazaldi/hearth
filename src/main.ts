import '@fontsource-variable/geist';
import '@fontsource-variable/geist-mono';
import './app.css';
import { mount } from 'svelte';

if (import.meta.env.DEV && !('__TAURI_INTERNALS__' in window)) await import('./lib/mock');
import App from './App.svelte';

export default mount(App, { target: document.getElementById('app')! });

