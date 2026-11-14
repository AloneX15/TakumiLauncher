<script>
  import { createEventDispatcher } from 'svelte';
  import { invoke } from '../lib/api.js';
  import { loaderName } from '../lib/format.js';
  import { launches, refreshInstances, settingsView, toast } from '../lib/stores.js';
  import Modal from './Modal.svelte';

  export let instance;

  const dispatch = createEventDispatcher();
  const global = $settingsView?.settings;

  // Empty fields mean "use the global setting".
  let name = instance.name;
  let memory = instance.settings.memory_mb ?? '';
  let javaPath = instance.settings.java_path ?? '';
  let jvmArgs = instance.settings.jvm_args ?? '';
  let width = instance.settings.width ?? '';
  let height = instance.settings.height ?? '';
  let saving = false;
  let confirmingDelete = false;

  $: busy = Boolean($launches[instance.id]);

  const numberOrNull = (value) => (value === '' || value == null ? null : Number(value));

  async function save() {
    saving = true;
    try {
      await invoke('update_instance', {
        id: instance.id,
        name,
        settings: {
          memory_mb: numberOrNull(memory),
          java_path: javaPath || null,
          jvm_args: jvmArgs || null,
          width: numberOrNull(width),
          height: numberOrNull(height),
        },
      });
      await refreshInstances();
      dispatch('close');
    } catch (error) {
      toast(error, 'error');
    } finally {
      saving = false;
    }
  }

  async function remove() {
    try {
      await invoke('delete_instance', { id: instance.id });
      await refreshInstances();
      dispatch('close');
    } catch (error) {
      toast(error, 'error');
    }
  }
</script>

<Modal title="Ajustes de la instancia" on:close>
  <p class="muted small summary">
    Minecraft {instance.minecraft_version}
    {#if instance.mod_loader !== 'vanilla'}· {loaderName(instance.mod_loader)} {instance.mod_loader_version}{/if}
  </p>

  <label class="field">
    <span>Nombre</span>
    <input bind:value={name} maxlength="64" />
  </label>

  <label class="field">
    <span>Memoria máxima (MB)</span>
    <input type="number" min="512" step="256" bind:value={memory} placeholder={`Global: ${global?.memory_mb ?? 4096}`} />
  </label>

  <div class="pair">
    <label class="field">
      <span>Ancho de ventana</span>
      <input type="number" min="320" bind:value={width} placeholder="Automático" />
    </label>
    <label class="field">
      <span>Alto de ventana</span>
      <input type="number" min="240" bind:value={height} placeholder="Automático" />
    </label>
  </div>

  <label class="field">
    <span>Ejecutable de Java</span>
    <input bind:value={javaPath} placeholder={global?.java_path || 'Automático (Java de Mojang)'} spellcheck="false" />
  </label>

  <label class="field">
    <span>Argumentos de JVM</span>
    <input bind:value={jvmArgs} placeholder={global?.jvm_args || 'Ninguno'} spellcheck="false" />
  </label>

  <svelte:fragment slot="footer">
    {#if confirmingDelete}
      <button class="danger" on:click={remove}>Sí, eliminar con sus mundos</button>
      <button on:click={() => (confirmingDelete = false)}>No</button>
    {:else}
      <button
        class="danger"
        disabled={busy}
        title={busy ? 'Detén el juego primero' : ''}
        on:click={() => (confirmingDelete = true)}
      >
        Eliminar
      </button>
    {/if}
    <div class="spacer" />
    <button on:click={() => dispatch('close')}>Cancelar</button>
    <button class="primary" disabled={saving || !name.trim()} on:click={save}>Guardar</button>
  </svelte:fragment>
</Modal>

<style>
  .summary {
    margin-bottom: 14px;
  }

  .pair {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 12px;
  }
</style>
