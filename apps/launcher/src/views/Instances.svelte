<script>
  import { createEventDispatcher } from 'svelte';
  import { invoke } from '../lib/api.js';
  import { describeLaunch, loaderName, timeAgo } from '../lib/format.js';
  import { accounts, attempt, instances, launch, launches, stop, toast } from '../lib/stores.js';
  import NewInstanceModal from '../components/NewInstanceModal.svelte';
  import InstanceSettingsModal from '../components/InstanceSettingsModal.svelte';

  const dispatch = createEventDispatcher();

  let creating = false;
  let editing = null;

  function play(instance) {
    if (!$accounts.active) {
      toast('Añade una cuenta antes de jugar.', 'error');
      dispatch('accounts');
      return;
    }
    launch(instance.id);
  }

  const openFolder = (id) => attempt(invoke('open_instance_folder', { id }));
</script>

<header class="row">
  <div>
    <h1>Instancias</h1>
    <p class="muted">Cada instancia tiene sus propios mundos, mods y ajustes.</p>
  </div>
  <div class="spacer" />
  <button class="primary" on:click={() => (creating = true)}>+ Nueva instancia</button>
</header>

{#if $instances.length === 0}
  <div class="card empty">
    <h2>Todavía no tienes instancias</h2>
    <p class="muted">Crea una para elegir la versión de Minecraft y el cargador de mods.</p>
    <button class="primary" on:click={() => (creating = true)}>Crear mi primera instancia</button>
  </div>
{:else}
  <div class="grid">
    {#each $instances as instance (instance.id)}
      {@const state = $launches[instance.id]}
      {@const progress = state && describeLaunch(state)}
      <article class="card instance" class:active={state}>
        <div class="top">
          <div class="tile">{instance.name[0].toUpperCase()}</div>
          <div class="info">
            <div class="title" title={instance.name}>{instance.name}</div>
            <div class="row tags">
              <span class="badge">{instance.minecraft_version}</span>
              {#if instance.mod_loader !== 'vanilla'}
                <span class="badge accent">{loaderName(instance.mod_loader)} {instance.mod_loader_version}</span>
              {/if}
            </div>
          </div>
        </div>

        {#if state}
          <div class="status">
            <div class="small label">{progress.label}</div>
            <div class="bar" class:indeterminate={progress.fraction === null && state.stage !== 'running'}>
              <div
                class="fill"
                class:running={state.stage === 'running'}
                style:width={state.stage === 'running' ? '100%' : `${(progress.fraction ?? 0) * 100}%`}
              />
            </div>
          </div>
        {:else}
          <div class="status muted small">
            {instance.last_played ? `Jugado ${timeAgo(instance.last_played)}` : 'Sin estrenar'}
          </div>
        {/if}

        <div class="row actions">
          {#if state}
            <button class="main" on:click={() => stop(instance.id)}>
              {state.stage === 'running' ? 'Detener' : 'Cancelar'}
            </button>
          {:else}
            <button class="primary main" on:click={() => play(instance)}>Jugar</button>
          {/if}
          <button title="Consola" on:click={() => dispatch('console', instance.id)}>Consola</button>
          <button title="Abrir carpeta" on:click={() => openFolder(instance.id)}>Carpeta</button>
          <button title="Ajustes" aria-label="Ajustes" on:click={() => (editing = instance)}>⚙</button>
        </div>
      </article>
    {/each}
  </div>
{/if}

{#if creating}
  <NewInstanceModal on:close={() => (creating = false)} />
{/if}
{#if editing}
  <InstanceSettingsModal instance={editing} on:close={() => (editing = null)} />
{/if}

<style>
  header {
    margin-bottom: 22px;
  }

  .empty {
    text-align: center;
    padding: 56px 24px;
  }

  .empty button {
    margin-top: 18px;
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(330px, 1fr));
    gap: 16px;
  }

  .instance {
    display: flex;
    flex-direction: column;
    gap: 14px;
    transition: border-color 0.15s;
  }

  .instance.active {
    border-color: rgba(220, 20, 60, 0.55);
  }

  .top {
    display: flex;
    gap: 14px;
    align-items: center;
  }

  .tile {
    width: 52px;
    height: 52px;
    border-radius: 12px;
    background: linear-gradient(145deg, var(--surface-3), var(--surface-2));
    border: 1px solid var(--border);
    display: grid;
    place-items: center;
    font-size: 22px;
    font-weight: 700;
    color: var(--accent-hover);
    flex-shrink: 0;
  }

  .info {
    min-width: 0;
  }

  .title {
    font-size: 16px;
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .tags {
    gap: 6px;
    margin-top: 5px;
    flex-wrap: wrap;
  }

  .status {
    min-height: 34px;
    display: flex;
    flex-direction: column;
    justify-content: center;
    gap: 6px;
  }

  .label {
    font-variant-numeric: tabular-nums;
  }

  .bar {
    height: 6px;
    border-radius: 3px;
    background: var(--surface-3);
    overflow: hidden;
  }

  .fill {
    height: 100%;
    background: var(--accent);
    border-radius: 3px;
    transition: width 0.2s;
  }

  .fill.running {
    background: var(--success);
  }

  .indeterminate .fill {
    width: 35% !important;
    animation: slide 1.1s ease-in-out infinite;
  }

  @keyframes slide {
    from {
      transform: translateX(-100%);
    }
    to {
      transform: translateX(290%);
    }
  }

  .actions {
    gap: 8px;
  }

  .actions button {
    padding: 7px 11px;
  }

  .actions .main {
    flex: 1;
  }
</style>
