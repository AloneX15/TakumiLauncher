<script>
  import { createEventDispatcher } from 'svelte';
  import { invoke, openExternal } from '../lib/api.js';
  import { describeLaunch, loaderName } from '../lib/format.js';
  import {
    accounts,
    appInfo,
    attempt,
    instances,
    launch,
    launches,
    privateInstances,
    privateUpdates,
    refreshInstances,
    restartPrivate,
    stop,
    syncPrivate,
    toast,
  } from '../lib/stores.js';
  import Modal from './Modal.svelte';
  import InstanceSettingsModal from './InstanceSettingsModal.svelte';

  const dispatch = createEventDispatcher();

  let joining = false;
  let link = '';
  let sending = false;
  let editing = null;

  $: remote = $privateInstances.list ?? [];
  $: remoteIds = new Set(remote.map((r) => r.id));
  // Downloaded before, but the user is no longer a member (or it was deleted).
  $: orphans = $instances.filter((i) => i.managed && !remoteIds.has(i.id) && $privateInstances.list);
  $: local = (id) => $instances.find((i) => i.id === id);

  function play(item) {
    if (!$accounts.active) {
      toast('Añade una cuenta antes de jugar.', 'error');
      dispatch('accounts');
      return;
    }
    launch(item.id);
  }

  async function join() {
    sending = true;
    try {
      await invoke('join_private_instance', { link });
      toast('Te has unido a la instancia.', 'success');
      joining = false;
      link = '';
      await refreshInstances();
    } catch (error) {
      toast(error, 'error');
    } finally {
      sending = false;
    }
  }

  async function forget(instance) {
    const done = await attempt(invoke('delete_instance', { id: instance.id }).then(() => true));
    if (done) refreshInstances();
  }
</script>

<section class="private">
  <div class="row head">
    <div>
      <div class="eyebrow">EVENTOS</div>
      <h2>Instancias privadas</h2>
    </div>
    <div class="spacer" />
    {#if $appInfo}
      <button class="ghost" on:click={() => openExternal(`${$appInfo.web_url}/instancias`)}>Gestionar en la web</button>
    {/if}
    <button on:click={() => (joining = true)}>Unirse con un enlace</button>
  </div>

  {#if $privateInstances.error && !$privateInstances.list}
    <p class="muted small">No se han podido cargar: {$privateInstances.error}</p>
  {:else if $privateInstances.list && remote.length === 0 && orphans.length === 0}
    <p class="muted small">
      No eres miembro de ninguna. Cuando el organizador de un evento te invite, pega aquí su enlace.
    </p>
  {/if}

  {#if remote.length || orphans.length}
    <div class="list">
      {#each remote as item (item.id)}
        {@const state = $launches[item.id]}
        {@const progress = state && describeLaunch(state)}
        {@const installed = local(item.id)}
        <article class="card item" class:active={state}>
          <div class="info">
            <div class="title" title={item.name}>{item.name}</div>
            <div class="row tags">
              <span class="badge">{item.minecraft}</span>
              {#if item.loader !== 'vanilla'}<span class="badge accent">{loaderName(item.loader)}</span>{/if}
              {#if item.version === 0}
                <span class="badge">Sin publicar</span>
              {:else if !item.installed_version}
                <span class="badge">No descargada</span>
              {:else if item.update_required}
                <span class="badge warning">Actualización obligatoria</span>
              {:else}
                <span class="badge success">v{item.version}</span>
              {/if}
              {#if item.server}<span class="muted small">{item.server}</span>{/if}
            </div>
            {#if item.description}<p class="muted small desc">{item.description}</p>{/if}
          </div>

          {#if state && $privateUpdates[item.id]}
            {@const update = $privateUpdates[item.id]}
            <div class="notice" role="alert">
              <strong>Actualización obligatoria (v{update.version})</strong>
              <span class="small">
                Reinicia el juego para evitar errores.
                {#if update.grace_minutes > 0}El servidor expulsará a quien siga con la versión anterior en {update.grace_minutes}
                  min.{:else}El servidor ya no acepta la versión anterior.{/if}
              </span>
              {#if update.notes}<span class="small muted">{update.notes}</span>{/if}
            </div>
          {/if}

          {#if state}
            <div class="status">
              <div class="small">{progress.label}</div>
              <div class="bar" class:indeterminate={progress.fraction === null && state.stage !== 'running'}>
                <div
                  class="fill"
                  class:running={state.stage === 'running'}
                  style:width={state.stage === 'running' ? '100%' : `${(progress.fraction ?? 0) * 100}%`}
                />
              </div>
            </div>
          {/if}

          <div class="row actions">
            {#if state && $privateUpdates[item.id]}
              <button class="primary main" on:click={() => restartPrivate(item.id)}>Reiniciar y actualizar</button>
            {:else if state}
              <button class="main" on:click={() => stop(item.id)}>
                {state.stage === 'running' ? 'Detener' : 'Cancelar'}
              </button>
            {:else if item.version === 0}
              <button class="main" disabled title="El organizador aún no la ha publicado">Jugar</button>
            {:else if item.update_required}
              <button class="primary main" on:click={() => syncPrivate(item.id)}>
                {item.installed_version ? 'Actualizar (obligatorio)' : 'Descargar'}
              </button>
            {:else}
              <button class="primary main" on:click={() => play(item)}>Jugar</button>
            {/if}
            <button on:click={() => dispatch('console', item.id)}>Consola</button>
            {#if installed}
              <button title="Ajustes" aria-label="Ajustes" on:click={() => (editing = installed)}>⚙</button>
            {/if}
          </div>
        </article>
      {/each}

      {#each orphans as instance (instance.id)}
        <article class="card item">
          <div class="info">
            <div class="title">{instance.name}</div>
            <p class="muted small">Ya no tienes acceso a esta instancia: te han quitado de ella o se ha eliminado.</p>
          </div>
          <div class="row actions">
            <button class="danger main" on:click={() => forget(instance)}>Borrar de este equipo</button>
          </div>
        </article>
      {/each}
    </div>
  {/if}
</section>

{#if joining}
  <Modal title="Unirse a una instancia privada" on:close={() => (joining = false)}>
    <p class="muted small">Pega el enlace de invitación que te ha pasado el organizador.</p>
    <label class="field">
      <span>Enlace</span>
      <input bind:value={link} placeholder="https://takumilabs.org/i/…" spellcheck="false" />
    </label>
    <svelte:fragment slot="footer">
      <div class="spacer" />
      <button on:click={() => (joining = false)}>Cancelar</button>
      <button class="primary" disabled={sending || !link.trim()} on:click={join}>
        {sending ? 'Uniéndote…' : 'Unirme'}
      </button>
    </svelte:fragment>
  </Modal>
{/if}

{#if editing}
  <InstanceSettingsModal instance={editing} on:close={() => (editing = null)} />
{/if}

<style>
  .notice {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 8px 10px;
    border-radius: 8px;
    border: 1px solid rgba(255, 176, 32, 0.45);
    background: rgba(255, 176, 32, 0.1);
  }
  .private {
    margin-bottom: 26px;
  }
  .head {
    margin-bottom: 12px;
    gap: 8px;
  }
  .eyebrow {
    color: var(--accent-hover);
    font-size: 10px;
    letter-spacing: 0.16em;
    font-weight: 650;
    margin-bottom: 4px;
  }
  h2 {
    font-size: 18px;
  }
  .list {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(min(100%, 350px), 1fr));
    gap: 14px;
  }
  .item {
    display: flex;
    flex-direction: column;
    gap: 12px;
    border-color: rgba(220, 20, 60, 0.25);
  }
  .item.active {
    border-color: rgba(220, 20, 60, 0.55);
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
  .desc {
    margin-top: 8px;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .status {
    display: flex;
    flex-direction: column;
    gap: 6px;
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
    margin-top: auto;
    padding-top: 12px;
    border-top: 1px solid var(--border);
  }
  .actions button {
    padding: 7px 11px;
  }
  .actions .main {
    flex: 1;
  }
</style>
