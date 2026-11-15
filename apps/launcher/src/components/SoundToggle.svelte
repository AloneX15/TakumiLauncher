<script>
  import { experience } from '../lib/experience.js';
  import Icon from './Icon.svelte';
</script>

<button
  class="ghost sound-toggle"
  data-sound="off"
  aria-label="Sonidos de la interfaz"
  aria-pressed={$experience.sound}
  title={$experience.sound ? 'Silenciar sonidos' : 'Activar sonidos'}
  on:click={() => experience.update((value) => ({ ...value, sound: !value.sound }))}
>
  <Icon name={$experience.sound ? 'sound' : 'muted'} size={16} />
  <span>Sonido {$experience.sound ? 'activado' : 'desactivado'}</span>
  <span class="indicator" class:enabled={$experience.sound}></span>
</button>

<style>
  .sound-toggle {
    display: flex;
    align-items: center;
    gap: 9px;
    color: var(--muted);
    font-size: 12px;
    padding: 9px 10px;
  }
  .indicator {
    width: 5px;
    height: 5px;
    border-radius: 50%;
    background: var(--muted);
    margin-left: auto;
  }
  .enabled {
    background: var(--accent);
    box-shadow: 0 0 8px var(--accent-soft);
  }
</style>
