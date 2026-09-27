<script lang="ts">
  import { nickHue } from '../nickcolor';

  let { nick, self = false, onnick }: { nick: string; self?: boolean; onnick: (nick: string, e: MouseEvent) => void } = $props();
</script>

<!-- Styled by MessageList, which also styles it by context (.who, .event, .card-head …). -->
<button class="nick" class:me={self} style="--h:{nickHue(nick)}" onclick={(e) => onnick(nick, e)}>{nick}</button>
