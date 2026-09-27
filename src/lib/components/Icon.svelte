<script lang="ts" module>
  // Hand-tuned 20px icons, 1.5px strokes, drawn on a 20-unit grid.
  const PATHS: Record<string, string> = {
    hash: 'M8 3.5 6.5 16.5M13.5 3.5 12 16.5M3.5 7.5h13M3 12.5h13',
    at: 'M13.5 10a3.5 3.5 0 1 1-7 0 3.5 3.5 0 0 1 7 0Zm0 0v1.25a2.25 2.25 0 0 0 4.5 0V10a8 8 0 1 0-3.2 6.4',
    user: 'M10 10a3.25 3.25 0 1 0 0-6.5 3.25 3.25 0 0 0 0 6.5ZM4 16.5c.8-2.6 3.1-4 6-4s5.2 1.4 6 4',
    users:
      'M7.5 9.5a2.75 2.75 0 1 0 0-5.5 2.75 2.75 0 0 0 0 5.5ZM2.5 16c.7-2.3 2.6-3.6 5-3.6s4.3 1.3 5 3.6M13 4.2a2.75 2.75 0 0 1 0 5.1M14.5 12.6c1.4.4 2.5 1.6 3 3.4',
    search: 'M9 15a6 6 0 1 0 0-12 6 6 0 0 0 0 12ZM17 17l-3.75-3.75',
    settings:
      'M3.5 6h8M15.5 6h1M3.5 14h1M8.5 14h8M13.5 8a2 2 0 1 0 0-4 2 2 0 0 0 0 4ZM6.5 16a2 2 0 1 0 0-4 2 2 0 0 0 0 4Z',
    plus: 'M10 4v12M4 10h12',
    x: 'M5 5l10 10M15 5 5 15',
    chevron: 'M7.5 5.5 12 10l-4.5 4.5',
    down: 'M5.5 8 10 12.5 14.5 8',
    clip: 'm15.5 9.5-5.6 5.6a3.5 3.5 0 0 1-5-5l6.2-6.2a2.3 2.3 0 0 1 3.3 3.3l-6.1 6.1a1.1 1.1 0 0 1-1.6-1.6L12.3 6',
    arrowDown: 'M10 4v12M5 11l5 5 5-5',
    server: 'M3.5 4.5h13v4.5h-13zM3.5 11h13v4.5h-13zM6.5 6.75h.01M6.5 13.25h.01',
    layers: 'm10 3 7 3.75L10 10.5 3 6.75 10 3ZM3 10.25 10 14l7-3.75M3 13.75 10 17.5l7-3.75',
    power: 'M10 3v6.5M6 5.5a6 6 0 1 0 8 0',
    refresh: 'M16 10a6 6 0 1 1-1.8-4.3M16 3.5v3h-3',
    edit: 'm12.5 4.5 3 3L7 16H4v-3l8.5-8.5Z',
    trash: 'M4 6h12M8 6V4h4v2M5.5 6l.8 10h7.4l.8-10',
    message: 'M4 4.5h12v8.5h-7l-3.5 3v-3H4z',
    info: 'M10 17a7 7 0 1 0 0-14 7 7 0 0 0 0 14ZM10 9v4.5M10 6.5h.01',
    bell: 'M5.5 13.5V9a4.5 4.5 0 0 1 9 0v4.5l1.5 1.5H4l1.5-1.5ZM8.5 17h3',
    sidebar: 'M3.5 4.5h13v11h-13zM12.5 4.5v11',
    check: 'm4.5 10.5 3.5 3.5 7.5-8',
    moon: 'M16 11.5A6.5 6.5 0 0 1 8.5 4a6.5 6.5 0 1 0 7.5 7.5Z',
    reply: 'M8 5 3.5 9.5 8 14M4 9.5h7a5 5 0 0 1 5 5v1',
    smile: 'M10 17a7 7 0 1 0 0-14 7 7 0 0 0 0 14ZM7 11.5s1 1.5 3 1.5 3-1.5 3-1.5M7.5 8h.01M12.5 8h.01',
    copy: 'M7 7h9v9H7zM4 13V4h9',
    list: 'M7 5.5h9.5M7 10h9.5M7 14.5h9.5M3.5 5.5h.01M3.5 10h.01M3.5 14.5h.01',
    eye: 'M2.5 10s2.75-5 7.5-5 7.5 5 7.5 5-2.75 5-7.5 5-7.5-5-7.5-5ZM10 12a2 2 0 1 0 0-4 2 2 0 0 0 0 4Z',
    bellOff: 'M5.5 13.5V9a4.5 4.5 0 0 1 7.3-3.5M14.5 8.5v5l1.5 1.5H6M8.5 17h3M3.5 3.5l13 13',
    lock: 'M5.5 9h9v7.5h-9zM7.5 9V6.5a2.5 2.5 0 0 1 5 0V9',
    external: 'M11.5 4H16v4.5M16 4l-7 7M13.5 11.5V16H4V6.5h4.5',
    image: 'M3.5 4.5h13v11h-13zM3.5 13l4-4 3.5 3.5 2-2 3.5 3.5M12.5 8h.01',
    play: 'M7 5v10l8-5-8-5Z',
    music: 'M8 14.5V4.5l8-1.5v10M8 14.5a2 2 0 1 1-4 0 2 2 0 0 1 4 0ZM16 13a2 2 0 1 1-4 0 2 2 0 0 1 4 0Z',
    key: 'M12.5 10a3.5 3.5 0 1 0-3.4-2.7L3.5 13v2.5H6v-1.5h1.5v-1.5H9l1.1-1.1c.7.4 1.5.6 2.4.6Z',
  };
</script>

<script lang="ts">
  let { name, size = 16, stroke = 1.5 }: { name: keyof typeof PATHS | string; size?: number; stroke?: number } = $props();
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 20 20"
  fill="none"
  stroke="currentColor"
  stroke-width={stroke}
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
>
  <path d={PATHS[name] ?? ''} />
</svg>
