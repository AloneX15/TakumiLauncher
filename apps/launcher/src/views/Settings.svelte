<script>
  import { invoke, openExternal } from '../lib/api.js';
  import { appInfo, attempt, settingsView, toast } from '../lib/stores.js';
  import { experience } from '../lib/experience.js';

  const AZURE_PORTAL =
    'https://portal.azure.com/#view/Microsoft_AAD_RegisteredApps/ApplicationsListBlade';

  // Edit a copy; the store only changes once the backend has saved it.
  let draft = { ...$settingsView.settings, java_path: $settingsView.settings.java_path ?? '' };
  let saving = false;

  $: dirty = JSON.stringify(normalise(draft)) !== JSON.stringify($settingsView.settings);

  function normalise(value) {
    return {
      ...value,
      memory_mb: Number(value.memory_mb) || 0,
      java_path: value.java_path.trim() || null,
      ms_client_id: value.ms_client_id.trim(),
    };
  }

  async function save() {
    saving = true;
    const saved = await attempt(invoke('save_settings', { settings: normalise(draft) }));
    saving = false;
    if (saved) {
      settingsView.set(saved);
      draft = { ...saved.settings, java_path: saved.settings.java_path ?? '' };
      toast('Ajustes guardados.', 'success');
    }
  }
</script>

<header class="row">
  <div>
    <h1>Ajustes</h1>
    <p class="muted">Valores por defecto para todas las instancias.</p>
  </div>
  <div class="spacer" />
  <button class="primary" disabled={!dirty || saving} on:click={save}>Guardar cambios</button>
</header>

<section class="card">
  <h2>Experiencia de la interfaz</h2>
  <p class="muted help">
    Pequeños detalles que acompañan cada interacción. Se guardan automáticamente en este
    dispositivo.
  </p>
  <label class="row check preference">
    <input type="checkbox" bind:checked={$experience.sound} />
    <span>Sonidos al interactuar</span>
  </label>
  <label class="field">
    <span>Volumen de los efectos: {$experience.volume}%</span>
    <input
      aria-label="Volumen de los efectos"
      type="range"
      min="0"
      max="100"
      step="5"
      disabled={!$experience.sound}
      bind:value={$experience.volume}
    />
  </label>
  <label class="row check">
    <input type="checkbox" bind:checked={$experience.reducedMotion} />
    <span
      >Reducir animaciones <span class="muted small"
        >· También respetamos la preferencia de tu sistema.</span
      ></span
    >
  </label>
</section>

<section class="card">
  <h2>Java y rendimiento</h2>

  <label class="field">
    <span>Memoria máxima: {draft.memory_mb} MB</span>
    <input type="range" min="1024" max="16384" step="512" bind:value={draft.memory_mb} />
  </label>

  <label class="field">
    <span>Ejecutable de Java (vacío: el launcher instala el Java adecuado para cada versión)</span>
    <input bind:value={draft.java_path} placeholder="Automático" spellcheck="false" />
  </label>

  <label class="field">
    <span>Argumentos de JVM adicionales</span>
    <input bind:value={draft.jvm_args} placeholder="-XX:+UseG1GC" spellcheck="false" />
  </label>

  <label class="row check">
    <input type="checkbox" bind:checked={draft.optimize_game} />
    <span>
      Optimizar el juego automáticamente
      <span class="muted small">
        — ajusta la memoria y el recolector de basura de Java y da prioridad al juego. Tus
        argumentos tienen preferencia.
      </span>
    </span>
  </label>
</section>

<section class="card">
  <h2>Versiones</h2>
  <label class="row check">
    <input type="checkbox" bind:checked={draft.show_snapshots} />
    <span>Mostrar snapshots y versiones antiguas al crear instancias</span>
  </label>
</section>

<section class="card">
  <h2>Inicio de sesión con Microsoft</h2>
  <p class="muted help">
    Microsoft exige que cada launcher use su propia aplicación registrada en Azure, con «flujos de
    cliente público» activados y el permiso de la API de Minecraft aprobado por Mojang. Pega aquí su
    ID de aplicación (cliente).
    <button class="ghost link" on:click={() => openExternal(AZURE_PORTAL)}
      >Abrir Azure Portal</button
    >
  </p>
  <label class="field">
    <span>ID de cliente</span>
    <input
      bind:value={draft.ms_client_id}
      placeholder="00000000-0000-0000-0000-000000000000"
      spellcheck="false"
      autocomplete="off"
    />
  </label>
  {#if !draft.ms_client_id.trim() && $settingsView.microsoft_login_available}
    <p class="muted small">Se está usando el ID definido en TAKUMI_MS_CLIENT_ID.</p>
  {/if}
</section>

{#if $appInfo}
  <section class="card">
    <h2>Datos del launcher</h2>
    <div class="row">
      <code class="selectable path">{$appInfo.data_dir}</code>
      <div class="spacer" />
      <button on:click={() => attempt(invoke('open_data_folder'))}>Abrir carpeta</button>
    </div>
  </section>
{/if}

<style>
  header {
    margin-bottom: 22px;
  }

  section {
    margin-bottom: 16px;
    max-width: 720px;
  }

  section :global(label.field:last-child) {
    margin-bottom: 0;
  }

  .check {
    cursor: pointer;
  }
  .preference {
    margin-bottom: 18px;
  }

  .help {
    margin-bottom: 14px;
  }

  .link {
    padding: 0 4px;
    color: var(--accent-hover);
  }

  .path {
    font-size: 12.5px;
    color: var(--muted);
    overflow-wrap: anywhere;
  }
</style>
