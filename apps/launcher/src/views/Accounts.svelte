<script>
  import { createEventDispatcher } from 'svelte';
  import { invoke, openExternal } from '../lib/api.js';
  import { accounts, attempt, deviceCode, session, settingsView, toast } from '../lib/stores.js';

  const dispatch = createEventDispatcher();

  let startingLogin = false;
  let confirmingRemoval = null;

  $: microsoftAvailable = $settingsView?.microsoft_login_available ?? false;
  $: hasMicrosoft = $accounts.accounts.some((a) => a.kind === 'microsoft');

  async function apply(promise) {
    const updated = await attempt(promise);
    if (updated) accounts.set(updated);
    return updated;
  }

  async function loginWithMicrosoft() {
    startingLogin = true;
    const code = await attempt(invoke('start_microsoft_login'));
    startingLogin = false;
    if (code) {
      deviceCode.set(code);
      openExternal(code.verification_uri);
    }
  }

  async function cancelLogin() {
    await attempt(invoke('cancel_microsoft_login'));
    deviceCode.set(null);
  }

  async function copyCode() {
    try {
      await navigator.clipboard.writeText($deviceCode.user_code);
      toast('Código copiado.', 'success');
    } catch {
      toast('No se pudo copiar; escríbelo a mano.', 'error');
    }
  }

  async function remove(id) {
    confirmingRemoval = null;
    await apply(invoke('remove_account', { id }));
  }
</script>

<header>
  <h1>Cuentas</h1>
  <p class="muted">
    La cuenta seleccionada es la que se usa al jugar. Si tienes una cuenta de Microsoft con Minecraft, se usa esa por
    defecto.
  </p>
</header>

{#if $accounts.accounts.length > 0}
  <section class="card list">
    {#each $accounts.accounts as account (account.id)}
      <div class="row account">
        <label class="row pick">
          <input
            type="radio"
            name="active"
            checked={$accounts.active === account.id}
            on:change={() => apply(invoke('set_active_account', { id: account.id }))}
          />
          <div class="avatar">{account.username[0].toUpperCase()}</div>
          <div>
            <div class="username">{account.username}</div>
            <div class="muted small">
              {account.kind === 'microsoft' ? 'Cuenta de Microsoft (premium)' : 'Sin conexión · tu usuario de Takumi'}
            </div>
          </div>
        </label>
        <div class="spacer" />
        {#if account.kind !== 'microsoft'}
          <span class="muted small">Automático</span>
        {:else if confirmingRemoval === account.id}
          <button class="danger" on:click={() => remove(account.id)}>Confirmar</button>
          <button on:click={() => (confirmingRemoval = null)}>Cancelar</button>
        {:else}
          <button class="ghost danger" on:click={() => (confirmingRemoval = account.id)}>Quitar</button>
        {/if}
      </div>
    {/each}
  </section>
{/if}

<section class="card">
  <h2>Cuenta de Microsoft</h2>
  {#if $deviceCode}
    <p class="muted">Introduce este código en la página de Microsoft que se ha abierto para autorizar el launcher:</p>
    <div class="code selectable">{$deviceCode.user_code}</div>
    <div class="row">
      <button on:click={copyCode}>Copiar código</button>
      <button on:click={() => openExternal($deviceCode.verification_uri)}>Abrir la página otra vez</button>
      <div class="spacer" />
      <span class="muted small">Esperando autorización…</span>
      <button on:click={cancelLogin}>Cancelar</button>
    </div>
  {:else if microsoftAvailable}
    <p class="muted">Inicia sesión con la cuenta que tiene Minecraft: Java Edition para jugar en línea.</p>
    <button class="primary" disabled={startingLogin} on:click={loginWithMicrosoft}>
      {startingLogin ? 'Conectando…' : 'Iniciar sesión con Microsoft'}
    </button>
  {:else}
    <p class="muted">
      Para iniciar sesión con Microsoft hace falta configurar el ID de cliente de la aplicación de Azure del launcher.
    </p>
    <button on:click={() => dispatch('settings')}>Configurar en Ajustes</button>
  {/if}
</section>

<section class="card">
  <h2>Perfil sin conexión</h2>
  <p class="muted">
    Si no tienes cuenta premium, juegas con tu usuario de Takumi
    {#if $session}<strong class="selectable">{$session.username}</strong>{/if}. Sirve para un jugador y para servidores
    sin verificación de cuentas, no para servidores en línea.
    {#if hasMicrosoft}Ahora mismo tiene prioridad tu cuenta de Microsoft.{/if}
  </p>
  <p class="muted small">El nombre es el de tu cuenta de Takumi Web y no se puede cambiar desde el launcher.</p>
</section>

<style>
  header {
    margin-bottom: 22px;
  }

  section {
    margin-bottom: 16px;
    max-width: 720px;
  }

  section p {
    margin-bottom: 14px;
  }

  .list {
    padding: 6px 18px;
  }

  .account {
    padding: 12px 0;
  }

  .account + .account {
    border-top: 1px solid var(--border);
  }

  .pick {
    cursor: pointer;
    gap: 12px;
  }

  .avatar {
    width: 36px;
    height: 36px;
    border-radius: 8px;
    background: var(--accent);
    color: #fff;
    display: grid;
    place-items: center;
    font-weight: 700;
  }

  .username {
    font-weight: 600;
  }

  .code {
    font:
      700 30px/1 'Cascadia Mono',
      Consolas,
      monospace;
    letter-spacing: 0.18em;
    text-align: center;
    padding: 18px;
    margin-bottom: 14px;
    background: var(--bg);
    border: 1px dashed var(--border);
    border-radius: var(--radius);
  }
</style>
