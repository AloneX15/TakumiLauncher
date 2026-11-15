<script>
  import { createEventDispatcher, onMount } from 'svelte';
  import { invoke } from '../lib/api.js';
  import { compactNumber, loaderName, megabytes } from '../lib/format.js';
  import { attempt, launches, toast } from '../lib/stores.js';

  export let instance;

  const dispatch = createEventDispatcher();
  const KINDS = [
    { id: 'mod', label: 'Mods' },
    { id: 'resourcepack', label: 'Paquetes de recursos' },
    { id: 'shader', label: 'Shaders' },
  ];

  let kind = instance.mod_loader === 'vanilla' ? 'resourcepack' : 'mod';
  let tab = 'installed';
  let items = [];
  let updates = {};
  let checking = false;
  let busy = {};

  let query = '';
  let results = null;
  let searching = false;
  let installing = {};

  $: running = Boolean($launches[instance.id]);
  $: visible = items.filter((item) => item.kind === kind);
  $: installedProjects = new Set(items.map((item) => item.project_id).filter(Boolean));
  $: modsUnavailable = kind === 'mod' && instance.mod_loader === 'vanilla';
  $: updateCount = Object.keys(updates).length;
  $: if (tab === 'browse') scheduleSearch(kind, query);

  const key = (item) => `${item.kind}/${item.filename}`;

  async function refresh() {
    const list = await attempt(invoke('list_content', { id: instance.id }));
    if (list) items = list;
  }

  onMount(refresh);

  async function withBusy(item, action) {
    busy = { ...busy, [key(item)]: true };
    try {
      await action();
    } catch (error) {
      toast(error, 'error');
    }
    const { [key(item)]: _done, ...rest } = busy;
    busy = rest;
    await refresh();
  }

  const args = (item) => ({ id: instance.id, kind: item.kind, filename: item.filename });

  const toggle = (item) =>
    withBusy(item, () => invoke('set_content_enabled', { ...args(item), enabled: !item.enabled }));

  const remove = (item) =>
    withBusy(item, async () => {
      await invoke('remove_content', args(item));
      forgetUpdate(item);
    });

  const update = (item) =>
    withBusy(item, async () => {
      await invoke('update_content', args(item));
      forgetUpdate(item);
      toast(`${item.title ?? item.filename} actualizado.`, 'success');
    });

  function forgetUpdate(item) {
    const { [key(item)]: _gone, ...rest } = updates;
    updates = rest;
  }

  async function checkUpdates() {
    checking = true;
    const found = await attempt(invoke('check_content_updates', { id: instance.id }));
    checking = false;
    if (!found) return;
    updates = Object.fromEntries(found.map((u) => [key(u), u.version_number]));
    toast(found.length ? `${found.length} actualización(es) disponible(s).` : 'Todo está al día.', 'success');
  }

  async function updateAll() {
    for (const item of items.filter((i) => updates[key(i)])) await update(item);
  }

  // Debounced search; stale responses are dropped.
  let timer;
  let request = 0;
  function scheduleSearch(currentKind, currentQuery) {
    clearTimeout(timer);
    if (currentKind === 'mod' && instance.mod_loader === 'vanilla') {
      results = null;
      return;
    }
    timer = setTimeout(() => search(currentKind, currentQuery, 0), 300);
  }

  async function search(currentKind, currentQuery, offset) {
    const id = ++request;
    searching = true;
    const page = await attempt(
      invoke('search_content', { id: instance.id, kind: currentKind, query: currentQuery, offset }),
    );
    if (id !== request) return;
    searching = false;
    if (page) results = offset === 0 ? page : { ...page, hits: [...results.hits, ...page.hits] };
  }

  let optimizing = false;
  async function optimize() {
    optimizing = true;
    try {
      const report = await invoke('optimize_instance', { id: instance.id });
      if (report.installed.length > 0) {
        toast(`Instalado: ${report.installed.join(', ')}`, 'success');
      } else if (report.already.length > 0) {
        toast('Esta instancia ya tiene los mods de rendimiento.', 'success');
      }
      if (report.unavailable.length > 0) {
        toast(`Sin versión para esta instancia todavía: ${report.unavailable.join(', ')}`);
      }
    } catch (error) {
      toast(error, 'error');
    }
    optimizing = false;
    await refresh();
  }

  async function install(hit) {
    installing = { ...installing, [hit.project_id]: true };
    try {
      const titles = await invoke('install_content', { id: instance.id, kind, project: hit.project_id });
      toast(titles.length > 1 ? `Instalado: ${titles.join(', ')}` : `${titles[0]} instalado.`, 'success');
    } catch (error) {
      toast(error, 'error');
    }
    const { [hit.project_id]: _done, ...rest } = installing;
    installing = rest;
    await refresh();
  }
</script>

<div class="page">
  <header class="row">
    <button data-sound="close" on:click={() => dispatch('back')}>← Volver</button>
    <div class="heading">
      <h1>{instance.name}</h1>
      <p class="muted small">
        Minecraft {instance.minecraft_version}
        {#if instance.mod_loader !== 'vanilla'}· {loaderName(instance.mod_loader)}{/if}
      </p>
    </div>
  </header>

  <div class="row bar">
    <div class="tabs">
      {#each KINDS as option}
        <button class:selected={kind === option.id} on:click={() => (kind = option.id)}>{option.label}</button>
      {/each}
    </div>
    <div class="spacer" />
    <div class="tabs">
      <button class:selected={tab === 'installed'} on:click={() => (tab = 'installed')}>Instalado</button>
      <button class:selected={tab === 'browse'} on:click={() => (tab = 'browse')}>Explorar Modrinth</button>
    </div>
  </div>

  {#if running}
    <p class="notice small">El juego está abierto: los cambios se aplicarán la próxima vez que lo inicies.</p>
  {/if}

  {#if modsUnavailable}
    <div class="card empty">
      <h2>Esta instancia es Vanilla</h2>
      <p class="muted">Los mods necesitan un cargador. Crea una instancia con Fabric, Quilt, Forge o NeoForge.</p>
    </div>
  {:else if tab === 'installed'}
    <div class="row tools">
      <span class="muted small">{visible.length} archivo(s)</span>
      <div class="spacer" />
      {#if updateCount > 0}
        <button class="primary" on:click={updateAll}>Actualizar todo ({updateCount})</button>
      {/if}
      {#if kind === 'mod'}
        <button
          disabled={optimizing}
          title="Instala los mods de rendimiento que falten: Sodium, Lithium, FerriteCore, Entity Culling e ImmediatelyFast"
          on:click={optimize}
        >
          {optimizing ? 'Optimizando…' : 'Optimizar rendimiento'}
        </button>
      {/if}
      <button disabled={checking || items.length === 0} on:click={checkUpdates}>
        {checking ? 'Buscando…' : 'Buscar actualizaciones'}
      </button>
    </div>

    {#if visible.length === 0}
      <div class="card empty">
        <h2>Nada instalado todavía</h2>
        <p class="muted">Busca contenido en Modrinth o copia archivos a la carpeta de la instancia.</p>
        <button class="primary" on:click={() => (tab = 'browse')}>Explorar Modrinth</button>
      </div>
    {:else}
      <div class="list">
        {#each visible as item (key(item))}
          <div class="card item" class:off={!item.enabled}>
            {#if item.icon_url}
              <img src={item.icon_url} alt="" loading="lazy" />
            {:else}
              <div class="placeholder">{(item.title ?? item.filename)[0].toUpperCase()}</div>
            {/if}
            <div class="text">
              <div class="title">{item.title ?? item.filename}</div>
              <div class="muted small ellipsis">
                {item.version_number ? `${item.version_number} · ` : ''}{megabytes(item.size)}
                {#if !item.title}· añadido a mano{/if}
              </div>
            </div>
            {#if updates[key(item)]}
              <button class="primary" disabled={busy[key(item)]} on:click={() => update(item)}>
                Actualizar a {updates[key(item)]}
              </button>
            {/if}
            <label class="row switch" title={item.enabled ? 'Desactivar' : 'Activar'}>
              <input type="checkbox" checked={item.enabled} disabled={busy[key(item)]} on:change={() => toggle(item)} />
              <span class="small">{item.enabled ? 'Activo' : 'Inactivo'}</span>
            </label>
            <button class="ghost danger" disabled={busy[key(item)]} on:click={() => remove(item)}>Eliminar</button>
          </div>
        {/each}
      </div>
    {/if}
  {:else}
    <input class="search" bind:value={query} placeholder="Buscar en Modrinth…" spellcheck="false" />
    {#if results}
      <div class="list">
        {#each results.hits as hit (hit.project_id)}
          <div class="card item">
            {#if hit.icon_url}
              <img src={hit.icon_url} alt="" loading="lazy" />
            {:else}
              <div class="placeholder">{hit.title[0].toUpperCase()}</div>
            {/if}
            <div class="text">
              <div class="title">{hit.title} <span class="muted small">de {hit.author}</span></div>
              <div class="muted small clamp">{hit.description}</div>
            </div>
            <span class="muted small downloads">{compactNumber(hit.downloads)} descargas</span>
            {#if installedProjects.has(hit.project_id)}
              <button disabled>Instalado</button>
            {:else}
              <button class="primary" disabled={installing[hit.project_id]} on:click={() => install(hit)}>
                {installing[hit.project_id] ? 'Instalando…' : 'Instalar'}
              </button>
            {/if}
          </div>
        {:else}
          <div class="card empty">
            <p class="muted">Sin resultados compatibles con Minecraft {instance.minecraft_version}.</p>
          </div>
        {/each}
      </div>
      {#if results.hits.length < results.total_hits}
        <button class="more" disabled={searching} on:click={() => search(kind, query, results.hits.length)}>
          {searching ? 'Cargando…' : 'Cargar más'}
        </button>
      {/if}
    {:else}
      <p class="muted">Buscando…</p>
    {/if}
  {/if}
</div>

<style>
  .page {
    display: flex;
    flex-direction: column;
    gap: 14px;
  }

  .heading {
    min-width: 0;
    margin-left: 6px;
  }

  h1 {
    font-size: 18px;
  }

  .tabs {
    display: flex;
    gap: 4px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 3px;
  }

  .tabs button {
    border-color: transparent;
    background: transparent;
    color: var(--muted);
    padding: 6px 12px;
  }

  .tabs button.selected {
    background: var(--accent-soft);
    color: var(--text);
  }

  .notice {
    color: #e8b84a;
  }

  .empty {
    text-align: center;
    padding: 40px 24px;
  }

  .empty button {
    margin-top: 16px;
  }

  .list {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .item {
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 12px 14px;
  }

  .item.off .text,
  .item.off img,
  .item.off .placeholder {
    opacity: 0.5;
  }

  img,
  .placeholder {
    width: 44px;
    height: 44px;
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
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .ellipsis {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .clamp {
    display: -webkit-box;
    -webkit-line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }

  .switch {
    gap: 6px;
    cursor: pointer;
    width: 84px;
  }

  .downloads {
    white-space: nowrap;
  }

  .more {
    align-self: center;
  }
</style>
