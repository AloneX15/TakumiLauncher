<script>
  import { afterUpdate, createEventDispatcher } from 'svelte';
  import { describeLaunch } from '../lib/format.js';
  import { launch, launches, logs, stop, toast } from '../lib/stores.js';

  export let instance;

  const dispatch = createEventDispatcher();
  let output;
  let follow = true;

  $: lines = $logs[instance.id] ?? [];
  $: state = $launches[instance.id];

  // Keep following the tail unless the user has scrolled up to read.
  function onScroll() {
    follow = output.scrollHeight - output.scrollTop - output.clientHeight < 40;
  }

  afterUpdate(() => {
    if (follow && output) output.scrollTop = output.scrollHeight;
  });

  async function copy() {
    try {
      await navigator.clipboard.writeText(lines.join('\n'));
      toast('Registro copiado.', 'success');
    } catch {
      toast('No se pudo copiar el registro.', 'error');
    }
  }

  const levelOf = (line) => /\b(ERROR|FATAL)\b|Exception|^\[Launcher\]/.test(line) ? 'error' : /\bWARN\b/.test(line) ? 'warn' : '';
</script>

<div class="console">
  <header class="row">
    <button data-sound="close" on:click={() => dispatch('back')}>← Volver</button>
    <div class="heading">
      <h1>{instance.name}</h1>
      <p class="muted small">{state ? describeLaunch(state).label : 'Detenido'}</p>
    </div>
    <div class="spacer" />
    <button disabled={lines.length === 0} on:click={copy}>Copiar registro</button>
    {#if state}
      <button on:click={() => stop(instance.id)}>{state.stage === 'running' ? 'Detener' : 'Cancelar'}</button>
    {:else}
      <button class="primary" on:click={() => launch(instance.id)}>Jugar</button>
    {/if}
  </header>

  <div class="output selectable" bind:this={output} on:scroll={onScroll}>
    {#each lines as line}
      <div class={levelOf(line)}>{line}</div>
    {:else}
      <div class="muted">La salida del juego aparecerá aquí cuando lo inicies.</div>
    {/each}
  </div>
</div>

<style>
  .console {
    display: flex;
    flex-direction: column;
    height: 100%;
    gap: 16px;
  }

  .heading {
    min-width: 0;
    margin-left: 6px;
  }

  h1 {
    font-size: 18px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .output {
    flex: 1;
    min-height: 0;
    overflow: auto;
    background: #08080b;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    padding: 12px 14px;
    font:
      12.5px/1.5 'Cascadia Mono',
      Consolas,
      monospace;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .error {
    color: var(--danger);
  }

  .warn {
    color: #e8b84a;
  }
</style>
