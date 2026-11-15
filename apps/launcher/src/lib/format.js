const relative = new Intl.RelativeTimeFormat('es', { numeric: 'auto' });

const UNITS = [
  ['year', 365 * 24 * 3600],
  ['month', 30 * 24 * 3600],
  ['day', 24 * 3600],
  ['hour', 3600],
  ['minute', 60],
];

/** "hace 3 horas", "ayer"… */
export function timeAgo(iso) {
  const seconds = (new Date(iso).getTime() - Date.now()) / 1000;
  for (const [unit, size] of UNITS) {
    if (Math.abs(seconds) >= size) return relative.format(Math.round(seconds / size), unit);
  }
  return 'hace un momento';
}

export function megabytes(bytes) {
  const mb = bytes / (1024 * 1024);
  return `${mb.toFixed(mb < 10 ? 1 : 0)} MB`;
}

const compact = new Intl.NumberFormat('es', { notation: 'compact', maximumFractionDigits: 1 });

/** 1234567 -> "1,2 M" */
export function compactNumber(value) {
  return compact.format(value);
}

/** Forge lists versions as "<minecraft>-<build>"; show only the build. */
export function loaderVersionLabel(version, minecraft) {
  return version.startsWith(`${minecraft}-`) ? version.slice(minecraft.length + 1) : version;
}

const LOADER_NAMES = {
  vanilla: 'Vanilla',
  fabric: 'Fabric',
  quilt: 'Quilt',
  forge: 'Forge',
  neoforge: 'NeoForge',
};

export function loaderName(loader) {
  return LOADER_NAMES[loader] ?? loader;
}

const STAGES = {
  auth: 'Iniciando sesión…',
  metadata: 'Preparando…',
  repair: 'Comprobando archivos…',
  downloading: 'Descargando',
  java: 'Instalando Java',
  loader: 'Instalando el cargador de mods',
  natives: 'Preparando librerías…',
  starting: 'Iniciando…',
  running: 'En ejecución',
};

/** Label and 0–1 fraction (null when indeterminate) for a launch state. */
export function describeLaunch(launch) {
  const label = STAGES[launch.stage] ?? launch.stage;
  const p = launch.progress;
  if (!p || p.files_total === 0) return { label, fraction: null };

  const byBytes = p.bytes_total > 0;
  const fraction = byBytes ? p.bytes_done / p.bytes_total : p.files_done / p.files_total;
  const detail = byBytes
    ? `${megabytes(p.bytes_done)} / ${megabytes(p.bytes_total)}`
    : `${p.files_done} / ${p.files_total}`;
  return { label: `${label} · ${detail}`, fraction: Math.min(1, Math.max(0, fraction)) };
}
