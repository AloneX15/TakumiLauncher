import { writable, get } from 'svelte/store';
import { invoke, listen } from './api.js';

const MAX_LOG_LINES = 3000;

export const instances = writable([]);
export const accounts = writable({ accounts: [], active: null });
/** `{ settings, microsoft_login_available }` */
export const settingsView = writable(null);
export const appInfo = writable(null);
/** Signed-in Takumi account: `{ username, email, role }` or null. */
export const session = writable(null);
/** Newer launcher release: `{ version, notes, download_url, page_url }` or null. */
export const launcherUpdate = writable(null);
/** Instance id -> `{ stage, progress }` while preparing or running. */
export const launches = writable({});
/** Instance id -> array of console lines. */
export const logs = writable({});
export const toasts = writable([]);
/** Pending Microsoft sign-in: `{ user_code, verification_uri }` or null. */
export const deviceCode = writable(null);

let toastId = 0;
export function toast(message, kind = 'info') {
  const id = ++toastId;
  toasts.update((list) => [...list, { id, message: String(message), kind }]);
  setTimeout(() => dismissToast(id), kind === 'error' ? 9000 : 4000);
}

export function dismissToast(id) {
  toasts.update((list) => list.filter((t) => t.id !== id));
}

/** Run a backend call, showing failures as an error toast. */
export async function attempt(promise) {
  try {
    return await promise;
  } catch (error) {
    toast(error, 'error');
    return undefined;
  }
}

/** The offline profile follows the Takumi account, so the list changes with the session. */
export async function refreshAccounts() {
  const list = await attempt(invoke('list_accounts'));
  if (list) accounts.set(list);
}

export async function refreshInstances() {
  const list = await attempt(invoke('list_instances'));
  if (list) instances.set(list);
  refreshPrivate();
}

/**
 * Private (event) instances of the signed-in Takumi user, from the website:
 * `{ list, error }`. `list` is null until loaded; on error the last list is kept.
 */
export const privateInstances = writable({ list: null, error: null });

export async function refreshPrivate() {
  try {
    const list = await invoke('list_private_instances');
    privateInstances.set({ list, error: null });
  } catch (error) {
    privateInstances.update((current) => ({ ...current, error: String(error) }));
  }
}

/**
 * Mandatory updates published while a private instance is running:
 * instance id -> `{ version, grace_minutes, deadline, notes }`. Cleared when the game exits.
 */
export const privateUpdates = writable({});

/** Close the running game of a private instance and start it again, updated. */
export async function restartPrivate(id) {
  try {
    await invoke('restart_private_instance', { id });
    toast('Cerrando el juego para actualizar…', 'info');
  } catch (error) {
    toast(error, 'error');
  }
}

/** Download or update a private instance without starting it. */
export async function syncPrivate(id) {
  launches.update((all) => ({ ...all, [id]: { stage: 'sync', progress: null } }));
  try {
    await invoke('sync_private_instance', { id });
    toast('Instancia privada al día.', 'success');
  } catch (error) {
    endLaunch(id);
    toast(error, 'error');
  }
  await refreshInstances();
}

export async function launch(id) {
  logs.update((all) => ({ ...all, [id]: [] }));
  launches.update((all) => ({ ...all, [id]: { stage: 'auth', progress: null } }));
  try {
    await invoke('launch_instance', { id });
  } catch (error) {
    endLaunch(id);
    toast(error, 'error');
  }
}

export function stop(id) {
  return attempt(invoke('stop_instance', { id }));
}

function endLaunch(id) {
  launches.update((all) => {
    const next = { ...all };
    delete next[id];
    return next;
  });
}

// Game output arrives line by line; batch it so a chatty game does not
// trigger a re-render per line.
const pendingLines = new Map();
let flushScheduled = false;

function queueLine(id, line) {
  if (!pendingLines.has(id)) pendingLines.set(id, []);
  pendingLines.get(id).push(line);
  if (!flushScheduled) {
    flushScheduled = true;
    setTimeout(flushLines, 150);
  }
}

function flushLines() {
  flushScheduled = false;
  logs.update((all) => {
    const next = { ...all };
    for (const [id, lines] of pendingLines) {
      const merged = [...(next[id] ?? []), ...lines];
      next[id] = merged.length > MAX_LOG_LINES ? merged.slice(-MAX_LOG_LINES) : merged;
    }
    return next;
  });
  pendingLines.clear();
}

function instanceName(id) {
  return get(instances).find((i) => i.id === id)?.name ?? 'El juego';
}

/** Load initial data and subscribe to backend events. */
export async function init() {
  await Promise.all([
    listen('launch-status', ({ instance_id, stage, progress }) => {
      launches.update((all) => ({ ...all, [instance_id]: { stage, progress } }));
    }),
    listen('launch-log', ({ instance_id, line }) => queueLine(instance_id, line)),
    listen('private-update', (update) => {
      privateUpdates.update((all) => ({ ...all, [update.instance_id]: update }));
      toast(
        `${instanceName(update.instance_id)}: hay una actualización obligatoria. Reinicia el juego para evitar errores.`,
        'error',
      );
      refreshPrivate();
    }),
    listen('launch-exit', ({ instance_id, code, error }) => {
      endLaunch(instance_id);
      privateUpdates.update(({ [instance_id]: _, ...rest }) => rest);
      if (error) {
        queueLine(instance_id, `[Launcher] ${error}`);
        toast(error, 'error');
      } else if (code !== null && code !== 0) {
        queueLine(instance_id, `[Launcher] El juego terminó con código ${code}.`);
        toast(`${instanceName(instance_id)} se cerró con un error (código ${code}). Revisa la consola.`, 'error');
      }
      refreshInstances();
    }),
    listen('auth-done', (updated) => {
      deviceCode.set(null);
      accounts.set(updated);
      toast('Sesión iniciada correctamente.', 'success');
    }),
    // The session is shown from cache at start-up and checked in the background.
    listen('session-changed', (updated) => {
      session.set(updated);
      refreshPrivate();
    }),
    listen('session-ended', () => {
      session.set(null);
      toast('Tu sesión de Takumi ha caducado o se ha cerrado. Vuelve a iniciar sesión.', 'error');
    }),
    listen('auth-error', (error) => {
      deviceCode.set(null);
      toast(error, 'error');
    }),
  ]);

  const [info, accountList, settings, running, current] = await Promise.all([
    attempt(invoke('get_app_info')),
    attempt(invoke('list_accounts')),
    attempt(invoke('get_settings')),
    attempt(invoke('running_instances')),
    attempt(invoke('get_session')),
    refreshInstances(),
  ]);
  if (info) appInfo.set(info);
  session.set(current ?? null);
  // Not awaited: the update notice must never delay startup.
  invoke('check_launcher_update')
    .then((update) => launcherUpdate.set(update ?? null))
    .catch(() => {});
  if (accountList) accounts.set(accountList);
  if (settings) settingsView.set(settings);
  // Survives a frontend reload while a game is still running.
  if (running?.length) {
    launches.update((all) => {
      const next = { ...all };
      for (const id of running) next[id] ??= { stage: 'running', progress: null };
      return next;
    });
  }
  // Sign-in, sign-out and a renamed Takumi account all change the offline profile.
  let lastUser = current?.username ?? null;
  session.subscribe((value) => {
    const user = value?.username ?? null;
    if (user !== lastUser) {
      lastUser = user;
      refreshAccounts();
    }
  });
}
