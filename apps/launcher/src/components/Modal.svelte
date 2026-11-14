<script>
  import { createEventDispatcher } from 'svelte';

  export let title;

  const dispatch = createEventDispatcher();
  const close = () => dispatch('close');
</script>

<svelte:window on:keydown={(e) => e.key === 'Escape' && close()} />

<!-- svelte-ignore a11y-click-events-have-key-events a11y-no-static-element-interactions -->
<div class="backdrop" on:mousedown|self={close}>
  <div class="dialog" role="dialog" aria-modal="true" aria-label={title}>
    <header>
      <h2>{title}</h2>
      <button class="ghost close" on:click={close} aria-label="Cerrar">✕</button>
    </header>
    <div class="body">
      <slot />
    </div>
    <footer>
      <slot name="footer" />
    </footer>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.6);
    display: grid;
    place-items: center;
    z-index: 50;
    padding: 24px;
  }

  .dialog {
    width: min(520px, 100%);
    max-height: 100%;
    display: flex;
    flex-direction: column;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 14px;
    box-shadow: 0 24px 60px rgba(0, 0, 0, 0.55);
  }

  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 16px 20px 0;
  }

  header h2 {
    margin: 0;
    font-size: 17px;
  }

  .close {
    padding: 4px 9px;
    color: var(--muted);
  }

  .body {
    padding: 18px 20px 6px;
    overflow-y: auto;
  }

  footer {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 12px 20px 18px;
  }
</style>
