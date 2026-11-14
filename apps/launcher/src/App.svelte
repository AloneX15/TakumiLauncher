<script>
  import { onMount } from 'svelte';
  import { isTauri } from './lib/api.js';
  import { accounts, appInfo, init, instances, toasts, dismissToast } from './lib/stores.js';
  import Instances from './views/Instances.svelte';
  import Accounts from './views/Accounts.svelte';
  import Settings from './views/Settings.svelte';
  import Console from './views/Console.svelte';

  const NAV = [
    { id: 'instances', label: 'Instancias' },
    { id: 'accounts', label: 'Cuentas' },
    { id: 'settings', label: 'Ajustes' },
  ];

  let view = 'instances';
  let consoleId = null;
  let ready = false;

  $: active = $accounts.accounts.find((a) => a.id === $accounts.active);
  $: consoleInstance = $instances.find((i) => i.id === consoleId);

  function openConsole(event) {
    consoleId = event.detail;
    view = 'console';
  }

  onMount(async () => {
    await init();
    ready = true;
  });
</script>

<div class="shell">
  <aside>
    <div class="brand">
      <div class="logo">T</div>
      <div>
        <div class="name">TAKUMI</div>
        <div class="muted small">Launcher</div>
      </div>
    </div>

    <nav>
      {#each NAV as item}
        <button
          class="ghost nav"
          class:current={view === item.id || (view === 'console' && item.id === 'instances')}
          on:click={() => (view = item.id)}
        >
          {item.label}
        </button>
      {/each}
    </nav>

    <div class="spacer" />

    <button class="ghost account" on:click={() => (view = 'accounts')}>
      {#if active}
        <div class="avatar">{active.username[0].toUpperCase()}</div>
        <div class="who">
          <div class="username">{active.username}</div>
          <div class="muted small">{active.kind === 'microsoft' ? 'Microsoft' : 'Sin conexión'}</div>
        </div>
      {:else}
        <div class="avatar empty">?</div>
        <div class="who">
          <div class="username">Sin cuenta</div>
          <div class="muted small">Añadir cuenta</div>
        </div>
      {/if}
    </button>
    {#if $appInfo}
      <div class="version muted small">v{$appInfo.version}</div>
    {/if}
  </aside>

  <main>
    {#if !isTauri}
      <div class="card notice">
        Esta interfaz necesita la aplicación de escritorio. Ejecútala con <code>cargo tauri dev</code>.
      </div>
    {/if}
    {#if ready}
      {#if view === 'instances'}
        <Instances on:console={openConsole} on:accounts={() => (view = 'accounts')} />
      {:else if view === 'accounts'}
        <Accounts on:settings={() => (view = 'settings')} />
      {:else if view === 'settings'}
        <Settings />
      {:else if view === 'console' && consoleInstance}
        <Console instance={consoleInstance} on:back={() => (view = 'instances')} />
      {:else}
        <Instances on:console={openConsole} on:accounts={() => (view = 'accounts')} />
      {/if}
    {/if}
  </main>
</div>

<div class="toasts">
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
    width: 216px;
    flex-shrink: 0;
    background: var(--surface);
    border-right: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    padding: 18px 12px 10px;
  }

  .brand {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 0 8px 22px;
  }

  .logo {
    width: 38px;
    height: 38px;
    border-radius: 10px;
    background: linear-gradient(145deg, var(--accent-hover), #8f0d27);
    display: grid;
    place-items: center;
    font-weight: 800;
    font-size: 20px;
    color: #fff;
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
    text-align: left;
    padding: 9px 12px;
    color: var(--muted);
  }

  .nav.current {
    background: var(--accent-soft);
    color: var(--text);
    box-shadow: inset 3px 0 0 var(--accent);
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

  .version {
    padding: 8px 8px 0;
  }

  main {
    flex: 1;
    min-width: 0;
    overflow-y: auto;
    padding: 26px 30px;
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
</style>
