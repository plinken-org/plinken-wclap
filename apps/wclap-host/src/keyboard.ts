/**
 * On-screen keyboard for the host page: the only way to play an instrument
 * where Web MIDI does not exist (Safari, so every iPad and iPhone).
 *
 * Pointer events with one note per pointer, so chords work under several
 * fingers and a finger sliding across keys plays a glissando. Velocity
 * comes from where the key is struck: near the front edge is loudest.
 */

export interface KeyboardOptions {
  /** Lowest key shown, a C (MIDI note number). */
  lowKey?: number;
  /** Number of octaves shown; one extra top C closes the range. */
  octaves?: number;
  onNoteOn: (key: number, velocity: number) => void;
  onNoteOff: (key: number) => void;
}

export interface Keyboard {
  /** Release every key the keyboard holds (panic, or the plugin went away). */
  releaseAll: () => void;
}

const BLACK = new Set([1, 3, 6, 8, 10]);
const MIN_LOW = 12;
const MAX_LOW = 84;

export function mountKeyboard(container: HTMLElement, opts: KeyboardOptions): Keyboard {
  const octaves = opts.octaves ?? 2;
  let low = opts.lowKey ?? 48;
  /** pointerId → the key that pointer currently holds. */
  const held = new Map<number, number>();

  container.classList.add('kbd');
  container.innerHTML = '';

  const bar = document.createElement('div');
  bar.className = 'kbdBar';
  const down = button('−', 'One octave lower');
  const up = button('+', 'One octave higher');
  const range = document.createElement('span');
  range.className = 'kbdRange';
  bar.appendChild(down);
  bar.appendChild(range);
  bar.appendChild(up);

  const keys = document.createElement('div');
  keys.className = 'kbdKeys';
  container.appendChild(bar);
  container.appendChild(keys);

  function button(text: string, title: string): HTMLButtonElement {
    const b = document.createElement('button');
    b.type = 'button';
    b.className = 'kbdOct';
    b.textContent = text;
    b.title = title;
    return b;
  }

  function build(): void {
    keys.innerHTML = '';
    const count = octaves * 12 + 1;
    const whites = Array.from({ length: count }, (_, i) => i).filter((i) => !BLACK.has(i % 12)).length;
    let white = 0;
    for (let i = 0; i < count; i++) {
      const key = low + i;
      const el = document.createElement('div');
      el.dataset.key = String(key);
      if (BLACK.has(i % 12)) {
        el.className = 'kbdKey kbdKey--black';
        // Centred on the boundary between its two white neighbours.
        el.style.left = `${(white / whites) * 100}%`;
        el.style.width = `${(0.62 / whites) * 100}%`;
      } else {
        el.className = 'kbdKey kbdKey--white';
        el.style.left = `${(white / whites) * 100}%`;
        el.style.width = `${(1 / whites) * 100}%`;
        if (i % 12 === 0) el.textContent = `C${Math.floor(key / 12) - 1}`;
        white++;
      }
      keys.appendChild(el);
    }
    range.textContent = `C${Math.floor(low / 12) - 1} – C${Math.floor(low / 12) - 1 + octaves}`;
    down.disabled = low <= MIN_LOW;
    up.disabled = low >= MAX_LOW;
  }

  function keyAt(x: number, y: number): { key: number; el: HTMLElement } | null {
    const el = document.elementFromPoint(x, y);
    if (!(el instanceof HTMLElement) || el.dataset.key === undefined || !keys.contains(el)) return null;
    return { key: Number(el.dataset.key), el };
  }

  function lit(key: number, on: boolean): void {
    keys.querySelector<HTMLElement>(`[data-key="${key}"]`)?.classList.toggle('kbdKey--down', on);
  }

  function isHeld(key: number): boolean {
    for (const k of held.values()) if (k === key) return true;
    return false;
  }

  function press(pointerId: number, x: number, y: number): void {
    const hit = keyAt(x, y);
    const prev = held.get(pointerId);
    if (hit?.key === prev) return;
    if (prev !== undefined) {
      held.delete(pointerId);
      // Another finger may still hold the same key.
      if (!isHeld(prev)) {
        lit(prev, false);
        opts.onNoteOff(prev);
      }
    }
    if (!hit) return;
    const already = isHeld(hit.key);
    held.set(pointerId, hit.key);
    if (already) return;
    const box = hit.el.getBoundingClientRect();
    const depth = Math.min(1, Math.max(0, (y - box.top) / box.height));
    lit(hit.key, true);
    opts.onNoteOn(hit.key, 0.35 + 0.65 * depth);
  }

  function release(pointerId: number): void {
    const key = held.get(pointerId);
    if (key === undefined) return;
    held.delete(pointerId);
    if (!isHeld(key)) {
      lit(key, false);
      opts.onNoteOff(key);
    }
  }

  function releaseAll(): void {
    const keysHeld = new Set(held.values());
    held.clear();
    for (const key of keysHeld) {
      lit(key, false);
      opts.onNoteOff(key);
    }
  }

  keys.addEventListener('pointerdown', (e) => {
    e.preventDefault();
    // Capture so the pointer keeps reporting to the keyboard while it
    // slides, and so its release is seen even off the keys.
    keys.setPointerCapture(e.pointerId);
    press(e.pointerId, e.clientX, e.clientY);
  });
  keys.addEventListener('pointermove', (e) => {
    if (held.has(e.pointerId) || keys.hasPointerCapture(e.pointerId)) press(e.pointerId, e.clientX, e.clientY);
  });
  const end = (e: PointerEvent): void => {
    release(e.pointerId);
    if (keys.hasPointerCapture(e.pointerId)) keys.releasePointerCapture(e.pointerId);
  };
  keys.addEventListener('pointerup', end);
  keys.addEventListener('pointercancel', end);
  // A long press must not open the text-selection or context menu.
  keys.addEventListener('contextmenu', (e) => e.preventDefault());

  const shift = (by: number): void => {
    releaseAll();
    low = Math.min(MAX_LOW, Math.max(MIN_LOW, low + by));
    build();
  };
  down.addEventListener('click', () => shift(-12));
  up.addEventListener('click', () => shift(12));

  build();
  return { releaseAll };
}
