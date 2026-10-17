<script lang="ts" module>
  import type { Component } from 'svelte';
  import Hash from '@lucide/svelte/icons/hash';
  import AtSign from '@lucide/svelte/icons/at-sign';
  import User from '@lucide/svelte/icons/user';
  import Users from '@lucide/svelte/icons/users';
  import Search from '@lucide/svelte/icons/search';
  import SlidersHorizontal from '@lucide/svelte/icons/sliders-horizontal';
  import Plus from '@lucide/svelte/icons/plus';
  import X from '@lucide/svelte/icons/x';
  import ChevronRight from '@lucide/svelte/icons/chevron-right';
  import ChevronDown from '@lucide/svelte/icons/chevron-down';
  import Paperclip from '@lucide/svelte/icons/paperclip';
  import ArrowDown from '@lucide/svelte/icons/arrow-down';
  import Server from '@lucide/svelte/icons/server';
  import Layers from '@lucide/svelte/icons/layers';
  import SquareTerminal from '@lucide/svelte/icons/square-terminal';
  import Power from '@lucide/svelte/icons/power';
  import RefreshCw from '@lucide/svelte/icons/refresh-cw';
  import Pencil from '@lucide/svelte/icons/pencil';
  import Trash2 from '@lucide/svelte/icons/trash-2';
  import MessageSquare from '@lucide/svelte/icons/message-square';
  import Info from '@lucide/svelte/icons/info';
  import Bell from '@lucide/svelte/icons/bell';
  import PanelRight from '@lucide/svelte/icons/panel-right';
  import Check from '@lucide/svelte/icons/check';
  import Moon from '@lucide/svelte/icons/moon';
  import Reply from '@lucide/svelte/icons/reply';
  import Smile from '@lucide/svelte/icons/smile';
  import Copy from '@lucide/svelte/icons/copy';
  import List from '@lucide/svelte/icons/list';
  import Eye from '@lucide/svelte/icons/eye';
  import BellOff from '@lucide/svelte/icons/bell-off';
  import Lock from '@lucide/svelte/icons/lock';
  import ExternalLink from '@lucide/svelte/icons/external-link';
  import Image from '@lucide/svelte/icons/image';
  import Play from '@lucide/svelte/icons/play';
  import Music from '@lucide/svelte/icons/music';
  import Key from '@lucide/svelte/icons/key-round';

  // Hearth's icon names, mapped onto lucide (shadcn's icon set).
  const ICONS: Record<string, Component<{ size?: number; strokeWidth?: number; 'aria-hidden'?: boolean }>> = {
    hash: Hash,
    at: AtSign,
    user: User,
    users: Users,
    search: Search,
    settings: SlidersHorizontal,
    plus: Plus,
    x: X,
    chevron: ChevronRight,
    down: ChevronDown,
    clip: Paperclip,
    arrowDown: ArrowDown,
    server: Server,
    layers: Layers,
    terminal: SquareTerminal,
    power: Power,
    refresh: RefreshCw,
    edit: Pencil,
    trash: Trash2,
    message: MessageSquare,
    info: Info,
    bell: Bell,
    sidebar: PanelRight,
    check: Check,
    moon: Moon,
    reply: Reply,
    smile: Smile,
    copy: Copy,
    list: List,
    eye: Eye,
    bellOff: BellOff,
    lock: Lock,
    external: ExternalLink,
    image: Image,
    play: Play,
    music: Music,
    key: Key,
  };
</script>

<script lang="ts">
  let { name, size = 16, stroke = 2 }: { name: string; size?: number; stroke?: number } = $props();
  const Glyph = $derived(ICONS[name]);
</script>

{#if Glyph}
  <Glyph {size} strokeWidth={stroke} aria-hidden={true} />
{/if}
