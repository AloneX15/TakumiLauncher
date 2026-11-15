<script>
  import { onMount } from 'svelte';
  import { invoke, openExternal } from '../lib/api.js';
  import { appInfo } from '../lib/stores.js';

  const CATEGORY = { news: 'Noticia', update: 'Actualización', blog: 'Blog' };
  const dateFormat = new Intl.DateTimeFormat('es-ES', { day: 'numeric', month: 'long', year: 'numeric' });

  let posts = [];
  let loading = true;
  let error = null;

  async function load() {
    loading = true;
    error = null;
    try {
      posts = await invoke('get_news');
    } catch (e) {
      error = String(e);
    }
    loading = false;
  }

  onMount(load);
</script>

<header class="row">
  <div>
    <h1>Noticias</h1>
    <p class="muted">Novedades y actualizaciones de Takumi.</p>
  </div>
  <div class="spacer" />
  <button on:click={load} disabled={loading}>Actualizar</button>
  {#if $appInfo}
    <button on:click={() => openExternal(`${$appInfo.web_url}/noticias`)}>Abrir en la web</button>
  {/if}
</header>

{#if loading && posts.length === 0}
  <p class="muted">Cargando noticias…</p>
{:else if error}
  <div class="card">
    <p>{error}</p>
    <button on:click={load}>Reintentar</button>
  </div>
{:else if posts.length === 0}
  <div class="card muted">Todavía no hay noticias publicadas.</div>
{:else}
  <div class="list">
    {#each posts as post (post.slug)}
      <button class="card post" on:click={() => openExternal(post.url)}>
        <div class="row meta">
          <span class="badge accent">{CATEGORY[post.category] ?? post.category}</span>
          {#if post.published_at}
            <span class="muted small">{dateFormat.format(new Date(post.published_at))}</span>
          {/if}
        </div>
        <h2>{post.title}</h2>
        {#if post.excerpt}<p class="muted">{post.excerpt}</p>{/if}
        <span class="more small">Leer más →</span>
      </button>
    {/each}
  </div>
{/if}

<style>
  header {
    margin-bottom: 22px;
    align-items: flex-end;
  }

  .list {
    display: flex;
    flex-direction: column;
    gap: 12px;
    max-width: 820px;
  }

  .post {
    display: block;
    width: 100%;
    text-align: left;
    cursor: pointer;
    transition: border-color 0.15s;
  }

  .post:hover:not(:disabled) {
    border-color: var(--accent);
    background: var(--surface);
  }

  .meta {
    margin-bottom: 8px;
  }

  h2 {
    margin-bottom: 4px;
  }

  .more {
    display: inline-block;
    margin-top: 10px;
    color: var(--accent-hover);
    font-weight: 600;
  }

  .card p {
    margin-bottom: 0;
  }
</style>
