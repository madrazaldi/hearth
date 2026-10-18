<script lang="ts">
  import { app, type Buffer } from '../state.svelte';
  import { nickHue } from '../nickcolor';
  import Avatar from './Avatar.svelte';
  import Icon from './Icon.svelte';
  import Shield from '@lucide/svelte/icons/shield';
  import ShieldOff from '@lucide/svelte/icons/shield-off';
  import Mic from '@lucide/svelte/icons/mic';
  import MicOff from '@lucide/svelte/icons/mic-off';
  import UserX from '@lucide/svelte/icons/user-x';
  import EyeOff from '@lucide/svelte/icons/eye-off';
  import * as DropdownMenu from '$lib/components/ui/dropdown-menu';
  import { Badge } from '$lib/components/ui/badge';

  let { buf, nick, x, y, onclose }: { buf: Buffer; nick: string; x: number; y: number; onclose: () => void } = $props();

  const conn = $derived(buf.conn);
  const member = $derived(buf.member(nick));
  const me = $derived(buf.member(conn.nick));
  const myRank = $derived(me?.prefix ? conn.prefix.symbols.indexOf(me.prefix[0]) : 99);
  const opIndex = $derived(conn.prefix.symbols.indexOf('@'));
  const canModerate = $derived(buf.kind === 'channel' && myRank <= (opIndex < 0 ? 0 : opIndex) && myRank !== 99);
  const isOp = $derived(!!member?.prefix.includes('@'));
  const isVoice = $derived(!!member?.prefix.includes('+'));
  const self = $derived(conn.isMe(nick));
  const source = $derived({ nick, user: member?.user, host: member?.host });
  const ignored = $derived(app.isIgnored(conn, source));
  const watched = $derived(app.isWatched(conn, nick));
  const online = $derived(conn.online.get(conn.fold(nick)));
  const profile = $derived(conn.profile(nick));
  const showAvatar = $derived(app.settings.loadAvatars && !!profile.avatar);
  const hasMeta = $derived(
    !!(member?.prefix || member?.account || member?.away || online !== undefined || ignored),
  );

  // Menus return focus to their trigger, which here is just a point on screen.
  const opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
  // Mention hands focus to the composer instead, so typing continues there.
  let toComposer = false;

  function mention() {
    toComposer = true;
    window.dispatchEvent(new CustomEvent('hearth:insert', { detail: `${nick}: ` }));
  }
</script>

<!-- The menu opens at a point, so a zero-size trigger stands in as its anchor. -->
<DropdownMenu.Root open onOpenChange={(o) => !o && onclose()}>
  <DropdownMenu.Trigger
    tabindex={-1}
    aria-hidden="true"
    class="pointer-events-none fixed size-0 opacity-0"
    style="left:{x}px; top:{y}px"
  />
  <DropdownMenu.Content
    side="bottom"
    align="start"
    sideOffset={8}
    collisionPadding={8}
    class="w-72 p-0"
    aria-labelledby="nick-menu-name"
    onclick={(e) => e.stopPropagation()}
    onCloseAutoFocus={(e) => {
      e.preventDefault();
      if (toComposer) window.dispatchEvent(new Event('hearth:focus-composer'));
      else if (opener?.isConnected) opener.focus({ preventScroll: true });
    }}
  >
    <div class="flex gap-3 border-b px-3.5 pt-3 pb-2.5">
      {#if showAvatar}<Avatar {conn} {nick} size={40} />{/if}
      <div class="min-w-0 flex-1">
        <div class="nick truncate text-[15px] leading-tight font-semibold" id="nick-menu-name" style="--h:{nickHue(nick)}">{nick}</div>
        {#if profile.displayName && profile.displayName !== nick}
          <div class="truncate text-[12.5px] text-muted-foreground">{profile.displayName}</div>
        {/if}
        {#if member?.user}
          <div class="selectable mono mt-0.5 text-[11.5px] break-all text-muted-foreground">{member.user}@{member.host}</div>
        {/if}
      </div>
    </div>
    {#if hasMeta || member?.realname}
      <div class="flex flex-col gap-1.5 border-b px-3.5 py-2.5">
        {#if hasMeta}
          <div class="flex flex-wrap gap-1">
            {#if member?.prefix}<Badge variant="secondary" class="mono">{member.prefix}</Badge>{/if}
            {#if online !== undefined}
              <Badge variant="outline" class={online ? 'text-ok' : 'text-muted-foreground'}>{online ? 'online' : 'offline'}</Badge>
            {/if}
            {#if member?.away}<Badge variant="outline" class="text-warn">away</Badge>{/if}
            {#if ignored}<Badge variant="outline" class="text-warn">ignored</Badge>{/if}
            {#if member?.account}<Badge variant="outline" class="text-muted-foreground">signed in as {member.account}</Badge>{/if}
          </div>
        {/if}
        {#if member?.realname}<div class="text-[12.5px] text-foreground/80">{member.realname}</div>{/if}
      </div>
    {/if}
    <div class="p-1">
      {#if !self}
        <DropdownMenu.Item onSelect={() => app.query(conn, nick)}><Icon name="message" size={15} />Message</DropdownMenu.Item>
        <DropdownMenu.Item onSelect={mention}><Icon name="at" size={15} />Mention</DropdownMenu.Item>
      {/if}
      <DropdownMenu.Item onSelect={() => conn.sendFrom(buf, `WHOIS ${nick} ${nick}`)}><Icon name="info" size={15} />Whois</DropdownMenu.Item>
      {#if !self && conn.canMonitor}
        <DropdownMenu.Item onSelect={() => (watched ? app.unwatch(conn, nick) : app.watch(conn, nick))}>
          <Icon name={watched ? 'bellOff' : 'bell'} size={15} />
          {watched ? 'Stop watching' : 'Tell me when online'}
        </DropdownMenu.Item>
      {/if}
      {#if !self}
        <DropdownMenu.Item onSelect={() => (ignored ? app.unignore(nick) : app.ignore(nick))}>
          {#if ignored}<Icon name="eye" size={15} />{:else}<EyeOff size={15} />{/if}
          {ignored ? 'Stop ignoring' : 'Ignore'}
        </DropdownMenu.Item>
      {/if}
      {#if canModerate && !self}
        <DropdownMenu.Separator />
        <DropdownMenu.Item onSelect={() => conn.send(`MODE ${buf.name} ${isOp ? '-' : '+'}o ${nick}`)}>
          {#if isOp}<ShieldOff size={15} />{:else}<Shield size={15} />{/if}
          {isOp ? 'Remove operator' : 'Make operator'}
        </DropdownMenu.Item>
        <DropdownMenu.Item onSelect={() => conn.send(`MODE ${buf.name} ${isVoice ? '-' : '+'}v ${nick}`)}>
          {#if isVoice}<MicOff size={15} />{:else}<Mic size={15} />{/if}
          {isVoice ? 'Remove voice' : 'Give voice'}
        </DropdownMenu.Item>
        <DropdownMenu.Item variant="destructive" onSelect={() => conn.send(`KICK ${buf.name} ${nick}`)}><UserX size={15} />Kick</DropdownMenu.Item>
      {/if}
    </div>
  </DropdownMenu.Content>
</DropdownMenu.Root>
