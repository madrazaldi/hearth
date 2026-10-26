<script lang="ts" module>
  import {
    Hash, AtSign, User, Users, Search, SlidersHorizontal, Plus, X, ChevronRight, ChevronDown, Paperclip,
    ArrowDown, Server, Layers, SquareTerminal, Power, RefreshCw, Pencil, Trash2, MessageSquare, Info, Bell,
    PanelRight, Check, Moon, Reply, Smile, Copy, List, Eye, BellOff, Lock, ExternalLink, Image, Play, Music,
    KeyRound, Flame, ChevronsUpDown, Keyboard, Settings, Ellipsis, ArrowUp, CheckCheck, Globe, Compass, Plug,
    Unplug, SmilePlus, Coffee, type IconNode,
  } from 'lucide';

  // Hearth's icon names, drawn with lucide (shadcn's icon set). The markup is
  // built once per icon: @lucide/svelte's components cost too much per
  // instance for the message list.
  const NODES: Record<string, IconNode> = {
    hash: Hash, at: AtSign, user: User, users: Users, search: Search, settings: SlidersHorizontal, plus: Plus,
    x: X, chevron: ChevronRight, down: ChevronDown, clip: Paperclip, arrowDown: ArrowDown, server: Server,
    layers: Layers, terminal: SquareTerminal, power: Power, refresh: RefreshCw, edit: Pencil, trash: Trash2,
    message: MessageSquare, info: Info, bell: Bell, sidebar: PanelRight, check: Check, moon: Moon, reply: Reply,
    smile: Smile, copy: Copy, list: List, eye: Eye, bellOff: BellOff, lock: Lock, external: ExternalLink,
    image: Image, play: Play, music: Music, key: KeyRound, flame: Flame, updown: ChevronsUpDown, keyboard: Keyboard,
    gear: Settings, more: Ellipsis, arrowUp: ArrowUp, read: CheckCheck, globe: Globe, compass: Compass, plug: Plug,
    unplug: Unplug, smilePlus: SmilePlus, coffee: Coffee,
  };

  const MARKUP: Record<string, string> = Object.fromEntries(
    Object.entries(NODES).map(([name, node]) => [
      name,
      node.map(([tag, attrs]) => `<${tag} ${Object.entries(attrs).map(([k, v]) => `${k}="${v}"`).join(' ')}/>`).join(''),
    ]),
  );
</script>

<script lang="ts">
  let { name, size = 16, stroke = 2 }: { name: string; size?: number; stroke?: number } = $props();
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width={stroke}
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
>
  {@html MARKUP[name] ?? ''}
</svg>
