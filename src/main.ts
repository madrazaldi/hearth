import '@fontsource/ibm-plex-sans/400.css';
import '@fontsource/ibm-plex-sans/400-italic.css';
import '@fontsource/ibm-plex-sans/500.css';
import '@fontsource/ibm-plex-sans/600.css';
import '@fontsource/ibm-plex-mono/400.css';
import '@fontsource/ibm-plex-mono/500.css';
import './app.css';
import { mount } from 'svelte';

if (import.meta.env.DEV && !('__TAURI_INTERNALS__' in window)) await import('./lib/mock');
import App from './App.svelte';

export default mount(App, { target: document.getElementById('app')! });

