<script>
  import { onMount } from 'svelte';
  import { invoke, isTauri } from '../lib/api.js';

  let checking = false;
  let result = null;
  let checkedAt = null;
  let mounted = false;

  $: connected = result?.status === 'connected';
  $: label = checking
    ? 'Comprobando web…'
    : connected
      ? 'Web conectada'
      : result?.status === 'error'
        ? 'Error en la web'
        : result?.status === 'unreachable'
          ? 'Web sin conexión'
          : 'Web sin comprobar';
  $: detail = checking
    ? 'Verificando conexión'
    : connected
      ? `${result.latency_ms} ms · Volver a comprobar`
      : 'Pulsar para comprobar';
  $: title = [
    result?.message,
    result?.url,
    checkedAt && `Última comprobación: ${checkedAt.toLocaleTimeString()}`,
    'Pulsa para comprobar la conexión de nuevo.',
  ]
    .filter(Boolean)
    .join('\n');

  async function check() {
    if (checking) return;
    checking = true;
    try {
      const response = isTauri
        ? await invoke('check_web_connection')
        : {
            status: 'unavailable',
            message: 'La comprobación necesita la aplicación de escritorio.',
          };
      if (mounted)
        result = response ?? {
          status: 'error',
          message: 'No se recibió un estado de conexión válido.',
        };
    } catch {
      if (mounted)
        result = {
          status: 'error',
          message: 'No se pudo comprobar el estado de la conexión. Vuelve a intentarlo.',
        };
    } finally {
      if (mounted) {
        checking = false;
        checkedAt = new Date();
      }
    }
  }

  function refreshIfVisible() {
    if (document.visibilityState === 'visible') void check();
  }

  onMount(() => {
    mounted = true;
    void check();
    const timer = setInterval(refreshIfVisible, 60000);
    window.addEventListener('focus', refreshIfVisible);
    window.addEventListener('online', refreshIfVisible);
    window.addEventListener('offline', refreshIfVisible);
    document.addEventListener('visibilitychange', refreshIfVisible);
    return () => {
      mounted = false;
      clearInterval(timer);
      window.removeEventListener('focus', refreshIfVisible);
      window.removeEventListener('online', refreshIfVisible);
      window.removeEventListener('offline', refreshIfVisible);
      document.removeEventListener('visibilitychange', refreshIfVisible);
    };
  });
</script>

<button
  class="connection"
  class:connected
  class:failed={result && !connected && !checking}
  {title}
  disabled={checking}
  aria-label={`Estado de conexión: ${label}. Volver a comprobar`}
  aria-busy={checking}
  on:click={check}
>
  <span class="dot" class:checking aria-hidden="true"></span>
  <span class="copy" aria-live="polite"
    ><span class="label">{label}</span><span class="detail">{detail}</span></span
  >
</button>

<style>
  .connection {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    padding: 10px;
    text-align: left;
    background: var(--surface);
  }
  .connection:disabled {
    opacity: 1;
  }
  .copy {
    display: flex;
    flex-direction: column;
    min-width: 0;
    gap: 2px;
  }
  .label {
    font-size: 12px;
    font-weight: 600;
  }
  .detail {
    font-size: 10px;
    color: var(--muted);
  }
  .dot {
    width: 7px;
    height: 7px;
    flex-shrink: 0;
    background: var(--muted);
    border-radius: 50%;
  }
  .connected .dot {
    background: var(--success);
    box-shadow: 0 0 10px rgba(62, 207, 142, 0.2);
  }
  .failed .dot {
    background: var(--danger);
  }
  .dot.checking {
    background: var(--accent);
    animation: checking 1s ease-in-out infinite;
  }
  @keyframes checking {
    50% {
      opacity: 0.35;
    }
  }
</style>
