<script>
  import { createEventDispatcher, onMount } from 'svelte';
  import { invoke } from '../lib/api.js';
  import { loaderName, loaderVersionLabel } from '../lib/format.js';
  import { attempt, refreshInstances, settingsView, toast } from '../lib/stores.js';
  import Modal from './Modal.svelte';

  const dispatch = createEventDispatcher();
  const LOADERS = ['vanilla', 'fabric', 'quilt', 'forge', 'neoforge'];
  const LATEST = '';

  let versions = [];
  let loadingVersions = true;
  let showSnapshots = $settingsView?.settings.show_snapshots ?? false;
  let version = '';
  let loader = 'vanilla';
  let loaderVersions = [];
  let loaderVersion = LATEST;
  let loadingLoader = false;
  let name = '';
  let nameEdited = false;
  let creating = false;

  $: visibleVersions = versions.filter((v) => v.kind === 'release' || showSnapshots || v.id === version);
  $: suggestedName = version ? (loader === 'vanilla' ? `Minecraft ${version}` : `${loaderName(loader)} ${version}`) : '';
  $: if (!nameEdited) name = suggestedName;
  $: loadLoaderVersions(loader, version);
  $: loaderUnavailable = loader !== 'vanilla' && !loadingLoader && loaderVersions.length === 0;
  $: canCreate = Boolean(version && name.trim()) && !creating && !loadingLoader && !loaderUnavailable;

  onMount(async () => {
    const list = await attempt(invoke('get_versions'));
    loadingVersions = false;
    if (list) {
      versions = list.versions;
      version = list.latest_release;
    }
  });

  // Guards against a slow response for a previous selection overwriting
  // the list for the current one.
  let request = 0;
  async function loadLoaderVersions(currentLoader, currentVersion) {
    const id = ++request;
    loaderVersions = [];
    loaderVersion = LATEST;
    if (currentLoader === 'vanilla' || !currentVersion) {
      loadingLoader = false;
      return;
    }
    loadingLoader = true;
    const list = await attempt(invoke('get_loader_versions', { loader: currentLoader, version: currentVersion }));
    if (id !== request) return;
    loaderVersions = list ?? [];
    loadingLoader = false;
  }

  async function create() {
    creating = true;
    try {
      await invoke('create_instance', {
        name,
        version,
        loader,
        loaderVersion: loaderVersion === LATEST ? null : loaderVersion,
      });
      await refreshInstances();
      dispatch('close');
    } catch (error) {
      toast(error, 'error');
    } finally {
      creating = false;
    }
  }
</script>

<Modal title="Nueva instancia" on:close>
  <label class="field">
    <span>Nombre</span>
    <input bind:value={name} on:input={() => (nameEdited = true)} maxlength="64" placeholder="Mi instancia" />
  </label>

  <label class="field">
    <span>Versión de Minecraft</span>
    <select bind:value={version} disabled={loadingVersions}>
      {#if loadingVersions}
        <option value="">Cargando versiones…</option>
      {/if}
      {#each visibleVersions as v (v.id)}
        <option value={v.id}>{v.id}{v.kind === 'release' ? '' : ` (${v.kind.replace('_', ' ')})`}</option>
      {/each}
    </select>
  </label>
  <label class="row check">
    <input type="checkbox" bind:checked={showSnapshots} />
    <span>Mostrar snapshots y versiones antiguas</span>
  </label>

  <div class="field-label">Cargador de mods</div>
  <div class="segmented">
    {#each LOADERS as option}
      <button class:selected={loader === option} on:click={() => (loader = option)}>{loaderName(option)}</button>
    {/each}
  </div>

  {#if loader !== 'vanilla'}
    <label class="field">
      <span>Versión de {loaderName(loader)}</span>
      <select bind:value={loaderVersion} disabled={loadingLoader || loaderUnavailable}>
        {#if loadingLoader}
          <option value={LATEST}>Cargando…</option>
        {:else if loaderUnavailable}
          <option value={LATEST}>No disponible para {version}</option>
        {:else}
          <option value={LATEST}>Última estable (recomendada)</option>
          {#each loaderVersions as v (v.version)}
            <option value={v.version}>{loaderVersionLabel(v.version, version)}</option>
          {/each}
        {/if}
      </select>
    </label>
  {/if}

  <svelte:fragment slot="footer">
    <div class="spacer" />
    <button on:click={() => dispatch('close')}>Cancelar</button>
    <button class="primary" disabled={!canCreate} on:click={create}>
      {creating ? 'Creando…' : 'Crear instancia'}
    </button>
  </svelte:fragment>
</Modal>

<style>
  .check {
    margin: -6px 0 16px;
    color: var(--muted);
    font-size: 13px;
    cursor: pointer;
  }

  .field-label {
    font-size: 12.5px;
    color: var(--muted);
    margin-bottom: 6px;
  }

  .segmented {
    display: grid;
    grid-template-columns: repeat(5, 1fr);
    gap: 6px;
    margin-bottom: 14px;
  }

  .segmented button {
    padding: 7px 4px;
  }

  .segmented button.selected {
    background: var(--accent-soft);
    border-color: var(--accent);
  }
</style>
