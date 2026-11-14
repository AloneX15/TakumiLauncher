import { writable, get } from 'svelte/store';
import { invoke, listen } from './api.js';

const MAX_LOG_LINES = 3000;

export const instances = writable([]);
export const accounts = writable({ accounts: [], active: null });
/** `{ settings, microsoft_login_available }` */
export const settingsView = writable(null);
export const appInfo = writable(null);
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

export async function refreshInstances() {
  const list = await attempt(invoke('list_instances'));
  if (list) instances.set(list);
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
    listen('launch-exit', ({ instance_id, code, error }) => {
      endLaunch(instance_id);
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
    listen('auth-error', (error) => {
      deviceCode.set(null);
      toast(error, 'error');
    }),
  ]);

  const [info, accountList, settings, running] = await Promise.all([
    attempt(invoke('get_app_info')),
    attempt(invoke('list_accounts')),
    attempt(invoke('get_settings')),
    attempt(invoke('running_instances')),
    refreshInstances(),
  ]);
  if (info) appInfo.set(info);
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
}
