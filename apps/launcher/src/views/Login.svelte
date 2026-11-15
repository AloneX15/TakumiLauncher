<script>
  import { invoke, openExternal } from '../lib/api.js';
  import { appInfo, attempt, session, toast } from '../lib/stores.js';
  import Logo from '../components/Logo.svelte';
  import SoundToggle from '../components/SoundToggle.svelte';
  import ConnectionStatus from '../components/ConnectionStatus.svelte';

  let user = '';
  let password = '';
  let signingIn = false;

  $: canSubmit = user.trim().length > 0 && password.length > 0 && !signingIn;
  $: webUrl = $appInfo?.web_url ?? 'http://localhost:5173';

  async function login() {
    signingIn = true;
    const result = await attempt(invoke('takumi_login', { user, password }));
    signingIn = false;
    if (result) {
      password = '';
      session.set(result);
    }
  }

  function loginWithMicrosoft() {
    toast('El inicio de sesión con Microsoft estará disponible próximamente.');
  }
</script>

<div class="screen">
  <div class="sound"><ConnectionStatus /><SoundToggle /></div>
  <div class="card panel">
    <div class="brand">
      <Logo size={44} />
      <div>
        <div class="name">TAKUMI</div>
        <div class="muted small">Launcher</div>
      </div>
    </div>

    <h1>Iniciar sesión</h1>
    <p class="muted">Entra con tu cuenta de Takumi para usar el launcher.</p>

    <form on:submit|preventDefault={login}>
      <label class="field">
        <span>Usuario o correo</span>
        <input bind:value={user} autocomplete="username" spellcheck="false" maxlength="64" />
      </label>
      <label class="field">
        <span>Contraseña</span>
        <input type="password" bind:value={password} autocomplete="current-password" />
      </label>
      <button class="primary wide" disabled={!canSubmit}>
        {signingIn ? 'Entrando…' : 'Iniciar sesión'}
      </button>
    </form>

    <div class="divider muted small"><span>o</span></div>

    <button class="wide" on:click={loginWithMicrosoft}>Iniciar sesión con Microsoft</button>

    <p class="register muted">
      ¿No tienes cuenta?
      <button class="link" on:click={() => openExternal(`${webUrl}/registro`)}>Regístrate</button>
    </p>
  </div>

  {#if $appInfo}
    <div class="version muted small">v{$appInfo.version}</div>
  {/if}
</div>

<style>
  .screen {
    position: relative;
    overflow-y: auto;
    height: 100%;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 14px;
    padding: 24px;
    background: radial-gradient(circle at 50% 0%, var(--accent-soft), transparent 60%), var(--bg);
  }

  .panel {
    width: 100%;
    max-width: 410px;
    padding: 34px;
    border-top: 2px solid var(--accent);
    box-shadow:
      0 24px 80px rgba(0, 0, 0, 0.35),
      0 0 70px var(--accent-soft);
    animation: panel-enter 500ms var(--ease-out);
  }
  .sound {
    width: 184px;
    position: absolute;
    top: 20px;
    right: 24px;
  }
  @keyframes panel-enter {
    from {
      opacity: 0;
      transform: translateY(14px);
    }
    to {
      opacity: 1;
      transform: translateY(0);
    }
  }

  .brand {
    display: flex;
    align-items: center;
    gap: 12px;
    margin-bottom: 24px;
  }

  .name {
    font-weight: 750;
    letter-spacing: 0.14em;
  }

  h1 + p {
    margin-bottom: 20px;
  }

  .wide {
    width: 100%;
    padding-top: 10px;
    padding-bottom: 10px;
  }

  .divider {
    display: flex;
    align-items: center;
    gap: 10px;
    margin: 16px 0;
  }

  .divider::before,
  .divider::after {
    content: '';
    flex: 1;
    border-top: 1px solid var(--border);
  }

  .register {
    margin: 20px 0 0;
    text-align: center;
  }

  .link {
    background: none;
    border: none;
    padding: 0;
    color: var(--accent-hover);
    font-weight: 600;
  }

  .link:hover:not(:disabled) {
    background: none;
    text-decoration: underline;
  }
</style>
