<script>
  import { createEventDispatcher, onDestroy, onMount } from 'svelte';
  import { invoke, listen, pickFile } from '../lib/api.js';
  import { compactNumber, megabytes } from '../lib/format.js';
  import { attempt, refreshInstances, toast } from '../lib/stores.js';
  import Modal from './Modal.svelte';

  const dispatch = createEventDispatcher();

  let query = '';
  let results = null;
  /** Title of the pack being installed, or null. */
  let installing = null;
  let progress = null;
  let unlisten = () => {};

  $: scheduleSearch(query);
  $: fraction = progress && progress.bytes_total > 0 ? progress.bytes_done / progress.bytes_total : null;

  onMount(async () => {
    unlisten = await listen('modpack-progress', (p) => (progress = p));
  });
  onDestroy(() => unlisten());

  let timer;
  let request = 0;
  function scheduleSearch(current) {
    clearTimeout(timer);
    timer = setTimeout(async () => {
      const id = ++request;
      const page = await attempt(invoke('search_modpacks', { query: current, offset: 0 }));
      if (id === request && page) results = page;
    }, 300);
  }

  async function run(title, command, args) {
    installing = title;
    progress = null;
    try {
      const instance = await invoke(command, args);
      await refreshInstances();
      toast(`Instancia «${instance.name}» creada.`, 'success');
      dispatch('close');
    } catch (error) {
      toast(error, 'error');
      installing = null;
    }
  }

  async function fromFile() {
    const path = await pickFile('Modpack de Modrinth', ['mrpack']);
    if (path) run(path.split(/[\\/]/).pop(), 'import_modpack', { path });
  }

  // Closing mid-install would hide the only progress indicator.
  const close = () => !installing && dispatch('close');
</script>

<Modal title="Instalar modpack" on:close={close}>
  {#if installing}
    <div class="progress">
      <p>Instalando <strong>{installing}</strong>…</p>
      <div class="bar" class:indeterminate={fraction === null}>
        <div class="fill" style:width={`${(fraction ?? 0) * 100}%`} />
      </div>
      <p class="muted small">
        {#if progress && progress.files_total > 0}
          {progress.files_done} / {progress.files_total} archivos
          {#if progress.bytes_total > 0}· {megabytes(progress.bytes_done)} / {megabytes(progress.bytes_total)}{/if}
        {:else}
          Preparando…
        {/if}
      </p>
    </div>
  {:else}
    <div class="row top">
      <input bind:value={query} placeholder="Buscar modpacks en Modrinth…" spellcheck="false" />
      <button on:click={fromFile}>Desde archivo…</button>
    </div>
    <div class="results">
      {#if results}
        {#each results.hits as hit (hit.project_id)}
          <div class="hit">
            {#if hit.icon_url}
              <img src={hit.icon_url} alt="" loading="lazy" />
            {:else}
              <div class="placeholder">{hit.title[0].toUpperCase()}</div>
            {/if}
            <div class="text">
              <div class="title">{hit.title}</div>
              <div class="muted small clamp">{hit.description}</div>
              <div class="muted small">{compactNumber(hit.downloads)} descargas</div>
            </div>
            <button class="primary" on:click={() => run(hit.title, 'install_modpack', { project: hit.project_id })}>
              Instalar
            </button>
          </div>
        {:else}
          <p class="muted">Sin resultados.</p>
        {/each}
      {:else}
        <p class="muted">Buscando…</p>
      {/if}
    </div>
    <p class="muted small note">
      Se instala la última versión del modpack. Los de CurseForge todavía no son compatibles.
    </p>
  {/if}
</Modal>

<style>
  .top {
    margin-bottom: 12px;
  }

  .top button {
    white-space: nowrap;
  }

  .results {
    display: flex;
    flex-direction: column;
    gap: 4px;
    height: 340px;
    overflow-y: auto;
    margin: 0 -6px;
    padding: 0 6px;
  }

  .hit {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 4px;
    border-bottom: 1px solid var(--border);
  }

  img,
  .placeholder {
    width: 46px;
    height: 46px;
    border-radius: 9px;
    flex-shrink: 0;
    object-fit: cover;
    background: var(--surface-3);
  }

  .placeholder {
    display: grid;
    place-items: center;
    font-weight: 700;
    color: var(--muted);
  }

  .text {
    flex: 1;
    min-width: 0;
  }

  .title {
    font-weight: 600;
  }

  .clamp {
    display: -webkit-box;
    -webkit-line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }

  .note {
    margin-top: 12px;
  }

  .progress {
    padding: 28px 0 20px;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .bar {
    height: 8px;
    border-radius: 4px;
    background: var(--surface-3);
    overflow: hidden;
  }

  .fill {
    height: 100%;
    background: var(--accent);
    border-radius: 4px;
    transition: width 0.2s;
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
</style>
