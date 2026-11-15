import { writable, get } from 'svelte/store';

const KEY = 'takumi-experience';
const defaults = { sound: true, volume: 35, reducedMotion: false };
let saved;
try {
  saved = JSON.parse(localStorage.getItem(KEY) ?? 'null');
} catch {
  // Preferences remain usable when storage is unavailable.
}

export const experience = writable({
  sound: typeof saved?.sound === 'boolean' ? saved.sound : defaults.sound,
  volume: Number.isFinite(saved?.volume)
    ? Math.min(100, Math.max(0, saved.volume))
    : defaults.volume,
  reducedMotion: saved?.reducedMotion === true,
});

let context;
let lastClick = 0;
let mounted = false;

/** Short, softly enveloped tones; no downloads or audio files required. */
async function playSound(kind) {
  const preferences = get(experience);
  if (!preferences.sound || !preferences.volume) return;
  // A real panel closure must not be swallowed by the click that preceded it.
  if (kind !== 'close' && performance.now() - lastClick < 55) return;
  lastClick = performance.now();
  try {
    const AudioContext = window.AudioContext ?? window.webkitAudioContext;
    if (!AudioContext) return;
    context ??= new AudioContext();
    if (context.state === 'suspended') await context.resume();
    if (!get(experience).sound || context.state !== 'running') return;
    const start = context.currentTime;
    const oscillator = context.createOscillator();
    const gain = context.createGain();
    oscillator.type = 'sine';
    const [from, to] = kind === 'close' ? [440, 180] : kind === 'primary' ? [740, 980] : [520, 390];
    oscillator.frequency.setValueAtTime(from, start);
    oscillator.frequency.exponentialRampToValueAtTime(to, start + 0.055);
    gain.gain.setValueAtTime(0, start);
    gain.gain.linearRampToValueAtTime((preferences.volume / 100) * 0.1, start + 0.004);
    gain.gain.exponentialRampToValueAtTime(0.0001, start + 0.075);
    oscillator.connect(gain);
    gain.connect(context.destination);
    oscillator.onended = () => {
      oscillator.disconnect();
      gain.disconnect();
    };
    oscillator.start(start);
    oscillator.stop(start + 0.08);
  } catch {
    // Audio must never interrupt an action, even without an output device.
  }
}

/** Called on actual modal teardown, so blocked close requests stay silent. */
export function playCloseSound() {
  if (mounted) void playSound('close');
}

export function mountExperience() {
  mounted = true;
  const unsubscribe = experience.subscribe((value) => {
    document.documentElement.dataset.reducedMotion = String(value.reducedMotion);
    try {
      localStorage.setItem(KEY, JSON.stringify(value));
    } catch {
      /* Optional persistence. */
    }
  });
  const onClick = (event) => {
    if (!event.isTrusted || !(event.target instanceof Element)) return;
    const control = event.target.closest(
      'button, a[href], input[type="checkbox"], input[type="radio"], select'
    );
    if (!control || control.matches(':disabled') || control.closest('[data-sound="off"]')) return;
    void playSound(
      control.dataset.sound === 'close'
        ? 'close'
        : control.classList.contains('primary')
          ? 'primary'
          : 'click'
    );
  };
  document.addEventListener('click', onClick, true);
  return () => {
    mounted = false;
    unsubscribe();
    document.removeEventListener('click', onClick, true);
    void context?.close().catch(() => {});
    context = undefined;
  };
}
