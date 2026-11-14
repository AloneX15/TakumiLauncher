// Thin wrapper over the Tauri global (enabled by `withGlobalTauri`), so the
// UI can also be opened in a plain browser during development.
const tauri = window.__TAURI__;

export const isTauri = Boolean(tauri);

export function invoke(command, args) {
  if (!tauri) return Promise.reject('Abre la interfaz desde la aplicación (cargo tauri dev).');
  return tauri.tauri.invoke(command, args);
}

/** Subscribe to a backend event; resolves to an unsubscribe function. */
export function listen(event, handler) {
  if (!tauri) return Promise.resolve(() => {});
  return tauri.event.listen(event, (e) => handler(e.payload));
}

export function openExternal(url) {
  return tauri ? tauri.shell.open(url) : window.open(url, '_blank');
}
