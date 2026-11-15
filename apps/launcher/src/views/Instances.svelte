<script>
  import { createEventDispatcher } from 'svelte';
  import { describeLaunch, loaderName, loaderVersionLabel, timeAgo } from '../lib/format.js';
  import { accounts, instances, launch, launches, stop, toast } from '../lib/stores.js';
  import NewInstanceModal from '../components/NewInstanceModal.svelte';
  import InstanceSettingsModal from '../components/InstanceSettingsModal.svelte';
  import ModpackModal from '../components/ModpackModal.svelte';
  import Icon from '../components/Icon.svelte';
  import PrivateInstances from '../components/PrivateInstances.svelte';

  const dispatch = createEventDispatcher();

  // Private instances are listed in their own section.
  $: own = $instances.filter((instance) => !instance.managed);

  let creating = false;
  let editing = null;
  let importing = false;

  function play(instance) {
    if (!$accounts.active) {
      toast('Añade una cuenta antes de jugar.', 'error');
      dispatch('accounts');
      return;
    }
    launch(instance.id);
  }
</script>

<header class="row">
  <div>
    <div class="eyebrow">TU ESPACIO DE JUEGO</div>
    <h1>Instancias <span class="count">{own.length}</span></h1>
    <p class="muted">Cada instancia tiene sus propios mundos, mods y ajustes.</p>
  </div>
  <div class="spacer" />
  <button on:click={() => (importing = true)}>Instalar modpack</button>
  <button class="primary" on:click={() => (creating = true)}>+ Nueva instancia</button>
</header>

<section class="welcome" aria-label="Tu biblioteca de Minecraft">
  <div class="welcome-copy">
    <div class="eyebrow">MINECRAFT, A TU MANERA</div>
    <h2>Tu próxima aventura<br />empieza aquí<span>.</span></h2>
    <p class="muted">Tus mundos, tus mods, tus reglas.<br />Todo preparado para volver a jugar.</p>
    <button class="ghost welcome-action" on:click={() => (creating = true)}
      >Crear una instancia <Icon name="arrow" size={16} /></button
    >
  </div>
  <div class="world" aria-hidden="true">
    <div class="orbit orbit-one"></div>
    <div class="orbit orbit-two"></div>
    <div class="world-cube"><Icon name="box" size={100} /></div>
    <span class="pixel pixel-one"></span><span class="pixel pixel-two"></span><span
      class="pixel pixel-three"
    ></span>
  </div>
</section>

<PrivateInstances on:console on:accounts />

{#if own.length === 0}
  <div class="card empty">
    <div class="empty-icon"><Icon name="instances" size={26} /></div>
    <h2>Todavía no tienes instancias</h2>
    <p class="muted">Crea una para elegir la versión de Minecraft y el cargador de mods.</p>
    <button class="primary" on:click={() => (creating = true)}>Crear mi primera instancia</button>
  </div>
{:else}
  <div class="grid">
    {#each own as instance, index (instance.id)}
      {@const state = $launches[instance.id]}
      {@const progress = state && describeLaunch(state)}
      <article
        class="card instance"
        class:active={state}
        style:--enter-delay={`${Math.min(index, 6) * 45}ms`}
      >
        <div class="top">
          <div class="tile">{instance.name[0].toUpperCase()}</div>
          <div class="info">
            <div class="title" title={instance.name}>{instance.name}</div>
            <div class="row tags">
              <span class="badge">{instance.minecraft_version}</span>
              {#if instance.mod_loader !== 'vanilla'}
                <span class="badge accent">
                  {loaderName(instance.mod_loader)}
                  {loaderVersionLabel(instance.mod_loader_version, instance.minecraft_version)}
                </span>
              {/if}
            </div>
          </div>
        </div>

        {#if state}
          <div class="status">
            <div class="small label">{progress.label}</div>
            <div
              class="bar"
              class:indeterminate={progress.fraction === null && state.stage !== 'running'}
            >
              <div
                class="fill"
                class:running={state.stage === 'running'}
                style:width={state.stage === 'running'
                  ? '100%'
                  : `${(progress.fraction ?? 0) * 100}%`}
              />
            </div>
          </div>
        {:else}
          <div class="status muted small">
            {instance.last_played ? `Jugado ${timeAgo(instance.last_played)}` : 'Sin estrenar'}
          </div>
        {/if}

        <div class="row actions">
          {#if state}
            <button class="main" on:click={() => stop(instance.id)}>
              {state.stage === 'running' ? 'Detener' : 'Cancelar'}
            </button>
          {:else}
            <button class="primary main" on:click={() => play(instance)}
              ><Icon name="play" size={14} /> Jugar</button
            >
          {/if}
          <button
            title="Mods, paquetes de recursos y shaders"
            on:click={() => dispatch('content', instance.id)}
          >
            Contenido
          </button>
          <button title="Consola" on:click={() => dispatch('console', instance.id)}>Consola</button>
          <button title="Ajustes" aria-label="Ajustes" on:click={() => (editing = instance)}
            >⚙</button
          >
        </div>
      </article>
    {/each}
  </div>
{/if}

{#if creating}
  <NewInstanceModal on:close={() => (creating = false)} />
{/if}
{#if importing}
  <ModpackModal on:close={() => (importing = false)} />
{/if}
{#if editing}
  <InstanceSettingsModal instance={editing} on:close={() => (editing = null)} />
{/if}

<style>
  header {
    margin-bottom: 22px;
  }
  .eyebrow {
    color: var(--muted);
    font-size: 10px;
    letter-spacing: 0.16em;
    font-weight: 650;
    margin-bottom: 7px;
  }
  h1 {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-bottom: 3px;
  }
  .count {
    font-size: 12px;
    letter-spacing: 0;
    border: 1px solid var(--border);
    border-radius: 7px;
    padding: 2px 8px;
    color: var(--muted);
  }
  .welcome {
    position: relative;
    isolation: isolate;
    display: flex;
    align-items: center;
    justify-content: space-between;
    min-height: 236px;
    overflow: hidden;
    border: 1px solid var(--border);
    border-radius: 16px;
    padding: 28px 32px;
    margin-bottom: 24px;
    background:
      radial-gradient(ellipse at 85% 60%, rgba(220, 20, 60, 0.17), transparent 65%),
      linear-gradient(120deg, var(--surface-2), var(--surface));
  }
  .welcome::after {
    content: '';
    position: absolute;
    inset: 0;
    z-index: -1;
    opacity: 0.2;
    background-image:
      linear-gradient(var(--border) 1px, transparent 1px),
      linear-gradient(90deg, var(--border) 1px, transparent 1px);
    background-size: 40px 40px;
    mask-image: linear-gradient(90deg, transparent 25%, black);
  }
  .welcome-copy {
    position: relative;
    z-index: 1;
  }
  .welcome .eyebrow {
    color: var(--accent-hover);
  }
  .welcome h2 {
    font-size: 30px;
    line-height: 1.15;
    letter-spacing: -0.04em;
    margin-bottom: 12px;
  }
  .welcome h2 span {
    color: var(--accent);
  }
  .welcome p {
    font-size: 13px;
    line-height: 1.7;
  }
  .welcome-action {
    display: flex;
    gap: 10px;
    align-items: center;
    margin: 14px 0 -5px -10px;
    font-size: 12px;
  }
  .world {
    position: relative;
    width: 240px;
    height: 180px;
    display: grid;
    place-items: center;
    flex-shrink: 0;
  }
  .world-cube {
    color: var(--accent);
    filter: drop-shadow(0 12px 28px rgba(220, 20, 60, 0.3));
    transform: rotate(-12deg);
    animation: float 7s ease-in-out infinite;
  }
  .orbit {
    position: absolute;
    width: 172px;
    height: 172px;
    border: 1px solid rgba(220, 20, 60, 0.2);
    border-radius: 32px;
    transform: rotate(30deg);
  }
  .orbit-two {
    width: 136px;
    height: 136px;
    transform: rotate(-12deg);
    border-color: rgba(220, 20, 60, 0.13);
  }
  .pixel {
    position: absolute;
    width: 7px;
    height: 7px;
    background: var(--accent);
    border-radius: 2px;
    opacity: 0.5;
  }
  .pixel-one {
    top: 15px;
    right: 36px;
    transform: rotate(20deg);
  }
  .pixel-two {
    bottom: 15px;
    left: 32px;
    width: 10px;
    height: 10px;
    transform: rotate(-15deg);
  }
  .pixel-three {
    right: 12px;
    bottom: 36px;
    width: 4px;
    height: 4px;
  }
  .empty-icon {
    width: 54px;
    height: 54px;
    border: 1px solid var(--border);
    background: var(--accent-soft);
    color: var(--accent-hover);
    border-radius: 15px;
    display: grid;
    place-items: center;
    margin: 0 auto 18px;
  }
  @keyframes float {
    0%,
    100% {
      transform: translateY(0) rotate(-12deg);
    }
    50% {
      transform: translateY(-8px) rotate(-8deg);
    }
  }
  @keyframes card-enter {
    from {
      opacity: 0;
      transform: translateY(12px);
    }
    to {
      opacity: 1;
      transform: translateY(0);
    }
  }

  .empty {
    text-align: center;
    padding: 56px 24px;
  }

  .empty button {
    margin-top: 18px;
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(min(100%, 350px), 1fr));
    gap: 16px;
  }

  .instance {
    display: flex;
    flex-direction: column;
    gap: 14px;
    transition:
      border-color 200ms ease,
      transform 240ms var(--ease-out),
      box-shadow 240ms ease;
    animation: card-enter 380ms var(--ease-out) backwards;
    animation-delay: var(--enter-delay, 0ms);
  }
  .instance:hover {
    border-color: rgba(220, 20, 60, 0.4);
    transform: translateY(-3px);
    box-shadow: 0 12px 28px rgba(0, 0, 0, 0.2);
  }

  .instance.active {
    border-color: rgba(220, 20, 60, 0.55);
  }

  .top {
    display: flex;
    gap: 14px;
    align-items: center;
  }

  .tile {
    width: 52px;
    height: 52px;
    border-radius: 12px;
    background: linear-gradient(145deg, var(--surface-3), var(--surface-2));
    border: 1px solid var(--border);
    display: grid;
    place-items: center;
    font-size: 22px;
    font-weight: 700;
    color: var(--accent-hover);
    flex-shrink: 0;
  }

  .info {
    min-width: 0;
  }

  .title {
    font-size: 16px;
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .tags {
    gap: 6px;
    margin-top: 5px;
    flex-wrap: wrap;
  }

  .status {
    min-height: 34px;
    display: flex;
    flex-direction: column;
    justify-content: center;
    gap: 6px;
  }

  .label {
    font-variant-numeric: tabular-nums;
  }

  .bar {
    height: 6px;
    border-radius: 3px;
    background: var(--surface-3);
    overflow: hidden;
  }

  .fill {
    height: 100%;
    background: var(--accent);
    border-radius: 3px;
    transition: width 0.2s;
  }

  .fill.running {
    background: var(--success);
  }

  .indeterminate .fill {
    width: 35% !important;
    animation: slide 1.1s ease-in-out infinite;
  }

  @keyframes slide {
    from {
      transform: translateX(-100%);
    }
    to {
      transform: translateX(290%);
    }
  }

  .actions {
    gap: 8px;
    margin-top: auto;
    flex-wrap: wrap;
    padding-top: 14px;
    border-top: 1px solid var(--border);
  }

  .actions button {
    padding: 7px 11px;
  }

  .actions .main {
    flex: 1;
    display: flex;
    justify-content: center;
    align-items: center;
    gap: 7px;
  }
  @media (max-width: 1050px) {
    header {
      flex-wrap: wrap;
    }
    header > div:first-child {
      width: 100%;
    }
    header > .spacer {
      display: none;
    }
    .welcome {
      padding: 24px;
    }
    .world {
      width: 180px;
    }
  }
</style>
