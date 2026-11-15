<script>
  import { onMount } from 'svelte';
  import { invoke, isTauri, openExternal } from './lib/api.js';
  import {
    accounts,
    appInfo,
    attempt,
    init,
    instances,
    launcherUpdate,
    session,
    toasts,
    dismissToast,
  } from './lib/stores.js';
  import Login from './views/Login.svelte';
  import Logo from './components/Logo.svelte';
  import Instances from './views/Instances.svelte';
  import Accounts from './views/Accounts.svelte';
  import Settings from './views/Settings.svelte';
  import Console from './views/Console.svelte';
  import Content from './views/Content.svelte';
  import News from './views/News.svelte';
  import Icon from './components/Icon.svelte';
  import SoundToggle from './components/SoundToggle.svelte';
  import ConnectionStatus from './components/ConnectionStatus.svelte';
  import { mountExperience } from './lib/experience.js';

  const NAV = [
    { id: 'instances', label: 'Instancias' },
    { id: 'news', label: 'Noticias' },
    { id: 'accounts', label: 'Cuentas' },
    { id: 'settings', label: 'Ajustes' },
  ];

  let view = 'instances';
  let consoleId = null;
  let contentId = null;
  let ready = false;

  $: active = $accounts.accounts.find((a) => a.id === $accounts.active);
  $: consoleInstance = $instances.find((i) => i.id === consoleId);
  $: contentInstance = $instances.find((i) => i.id === contentId);
  $: inInstances = ['instances', 'console', 'content'].includes(view);

  function openContent(event) {
    contentId = event.detail;
    view = 'content';
  }

  function openConsole(event) {
    consoleId = event.detail;
    view = 'console';
  }

  async function logout() {
    // `attempt` yields undefined only on failure; a successful logout resolves to null.
    if ((await attempt(invoke('takumi_logout'))) !== undefined) {
      session.set(null);
      view = 'instances';
    }
  }

  onMount(() => {
    const cleanup = mountExperience();
    void init().then(() => {
      ready = true;
    });
    return cleanup;
  });
</script>

{#if !ready}
  <!-- Checking the session with the server: a splash instead of flashing the shell. -->
  <div class="splash"><Logo size={72} /></div>
{:else if !$session}
  <Login />
{:else}
  <div class="shell">
    <aside>
      <div class="brand">
        <Logo size={40} />
        <div>
          <div class="name">TAKUMI</div>
          <div class="muted small">Launcher</div>
        </div>
      </div>

      <nav>
        {#each NAV as item}
          <button
            class="ghost nav"
            class:current={view === item.id || (inInstances && item.id === 'instances')}
            aria-current={view === item.id || (inInstances && item.id === 'instances')
              ? 'page'
              : undefined}
            on:click={() => (view = item.id)}
          >
            <Icon name={item.id} /><span>{item.label}</span>
            {#if view === item.id || (inInstances && item.id === 'instances')}<span class="nav-dot"
              ></span>{/if}
          </button>
        {/each}
      </nav>

      <div class="spacer" />

      <div class="sound-control"><ConnectionStatus /><SoundToggle /></div>

      {#if $launcherUpdate}
        <button class="update" on:click={() => openExternal($launcherUpdate.download_url)}>
          <span class="update-title">Nueva versión {$launcherUpdate.version}</span>
          <span class="small">Descárgala desde la web</span>
        </button>
      {/if}

      <button class="ghost account" on:click={() => (view = 'accounts')}>
        {#if active}
          <div class="avatar">{active.username[0].toUpperCase()}</div>
          <div class="who">
            <div class="username">{active.username}</div>
            <div class="muted small">
              {active.kind === 'microsoft' ? 'Microsoft' : 'Sin conexión'}
            </div>
          </div>
        {:else}
          <div class="avatar empty">?</div>
          <div class="who">
            <div class="username">Sin cuenta</div>
            <div class="muted small">Añadir cuenta</div>
          </div>
        {/if}
      </button>
      {#if $session}
        <div class="row takumi">
          <button
            class="link small takumi-user"
            title="Abrir mi cuenta en la web"
            on:click={() => openExternal(`${$appInfo?.web_url}/cuenta`)}>{$session.username}</button
          >
          <div class="spacer" />
          <button class="ghost small" on:click={logout}>Cerrar sesión</button>
        </div>
      {/if}
      {#if $appInfo}
        <div class="version muted small">v{$appInfo.version}</div>
      {/if}
    </aside>

    <main>
      {#if !isTauri}
        <div class="card notice">
          Esta interfaz necesita la aplicación de escritorio. Ejecútala con <code
            >cargo tauri dev</code
          >.
        </div>
      {/if}
      {#if ready}
        {#key view}
          <div class="view-content">
            {#if view === 'accounts'}
              <Accounts on:settings={() => (view = 'settings')} />
            {:else if view === 'news'}
              <News />
            {:else if view === 'settings'}
              <Settings />
            {:else if view === 'console' && consoleInstance}
              <Console instance={consoleInstance} on:back={() => (view = 'instances')} />
            {:else if view === 'content' && contentInstance}
              {#key contentInstance.id}
                <Content instance={contentInstance} on:back={() => (view = 'instances')} />
              {/key}
            {:else}
              <Instances
                on:console={openConsole}
                on:content={openContent}
                on:accounts={() => (view = 'accounts')}
              />
            {/if}
          </div>
        {/key}
      {/if}
    </main>
  </div>
{/if}

<div class="toasts" aria-live="polite" aria-relevant="additions">
  {#each $toasts as item (item.id)}
    <button class="toast {item.kind}" on:click={() => dismissToast(item.id)}>{item.message}</button>
  {/each}
</div>

<style>
  .shell {
    display: flex;
    height: 100%;
  }

  aside {
    width: 224px;
    flex-shrink: 0;
    background: linear-gradient(180deg, var(--surface), var(--bg));
    border-right: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    padding: 28px 14px 14px;
  }

  .brand {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 0 8px 32px;
  }

  .name {
    font-weight: 750;
    letter-spacing: 0.14em;
  }

  nav {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .nav {
    display: flex;
    align-items: center;
    gap: 12px;
    text-align: left;
    padding: 12px;
    color: var(--muted);
  }

  .nav.current {
    background: var(--accent-soft);
    color: var(--text);
    box-shadow: inset 3px 0 0 var(--accent);
  }

  .nav-dot {
    width: 5px;
    height: 5px;
    border-radius: 50%;
    background: var(--accent);
    margin-left: auto;
  }
  .sound-control {
    border-bottom: 1px solid var(--border);
    padding-bottom: 12px;
    margin-bottom: 12px;
  }
  .sound-control :global(button) {
    width: 100%;
  }
  .view-content {
    animation: view-enter 320ms var(--ease-out);
  }
  @keyframes view-enter {
    from {
      opacity: 0;
      transform: translateY(8px);
    }
    to {
      opacity: 1;
      transform: translateY(0);
    }
  }

  .account {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px;
    text-align: left;
  }

  .avatar {
    width: 34px;
    height: 34px;
    border-radius: 8px;
    background: var(--accent);
    color: #fff;
    display: grid;
    place-items: center;
    font-weight: 700;
    flex-shrink: 0;
  }

  .avatar.empty {
    background: var(--surface-3);
    color: var(--muted);
  }

  .who {
    min-width: 0;
  }

  .username {
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .takumi {
    padding: 4px 0 0 8px;
    gap: 6px;
  }

  .splash {
    height: 100%;
    display: grid;
    place-items: center;
    animation: pulse 1.4s ease-in-out infinite;
  }

  @keyframes pulse {
    50% {
      opacity: 0.45;
    }
  }

  .update {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 2px;
    margin: 0 0 10px;
    padding: 10px 12px;
    text-align: left;
    background: var(--accent-soft);
    border: 1px solid var(--accent);
    color: var(--text);
  }

  .update:hover:not(:disabled) {
    background: rgba(220, 20, 60, 0.24);
  }

  .update-title {
    font-weight: 700;
  }

  .takumi .takumi-user {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    background: none;
    border: none;
    padding: 0;
    color: var(--muted);
    font-weight: 500;
    text-align: left;
  }

  .takumi .takumi-user:hover:not(:disabled) {
    background: none;
    color: var(--text);
    text-decoration: underline;
  }

  .takumi button {
    padding: 4px 8px;
    flex-shrink: 0;
  }

  .version {
    padding: 8px 8px 0;
  }

  main {
    flex: 1;
    min-width: 0;
    overflow-y: auto;
    padding: 32px;
    background: radial-gradient(ellipse at 90% 0%, var(--accent-soft), transparent 48%);
  }

  .notice {
    margin-bottom: 18px;
  }

  .toasts {
    position: fixed;
    right: 18px;
    bottom: 18px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    max-width: 420px;
    z-index: 100;
  }

  .toast {
    animation: toast-enter 240ms var(--ease-out);
    text-align: left;
    background: var(--surface-2);
    border-left: 3px solid var(--muted);
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.45);
    padding: 11px 14px;
    overflow-wrap: anywhere;
  }

  .toast.error {
    border-left-color: var(--danger);
  }

  .toast.success {
    border-left-color: var(--success);
  }
  @keyframes toast-enter {
    from {
      opacity: 0;
      transform: translateY(10px);
    }
    to {
      opacity: 1;
      transform: translateY(0);
    }
  }
</style>
