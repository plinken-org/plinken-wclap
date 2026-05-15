/**
 * Tiny DOM helpers for the WCLAP host page. No framework — just typed
 * `getElementById` lookups and explicit element updates.
 */

function el<T extends HTMLElement>(id: string): T {
  const node = document.getElementById(id);
  if (!node) throw new Error(`#${id} not found in DOM`);
  return node as T;
}

export interface UiElements {
  rack: HTMLElement;
  shelf: HTMLElement;
  statusLabel: HTMLElement;
  pluginLabel: HTMLElement;
  sampleRateLabel: HTMLElement;
  coiLabel: HTMLElement;
  audioStateLabel: HTMLElement;
  playBtn: HTMLButtonElement;
  stopBtn: HTMLButtonElement;
  meterL: HTMLElement;
  meterR: HTMLElement;
  errorBox: HTMLPreElement;
  midiLed: HTMLElement;
  midiStatus: HTMLElement;
  midiNotes: HTMLElement;
  midiPanic: HTMLButtonElement;
  midiRescan: HTMLButtonElement;
}

export function getElements(): UiElements {
  return {
    rack: el<HTMLElement>('rack'),
    shelf: el<HTMLElement>('shelf'),
    statusLabel: el<HTMLElement>('statusLabel'),
    pluginLabel: el<HTMLElement>('pluginLabel'),
    sampleRateLabel: el<HTMLElement>('sampleRateLabel'),
    coiLabel: el<HTMLElement>('coiLabel'),
    audioStateLabel: el<HTMLElement>('audioStateLabel'),
    playBtn: el<HTMLButtonElement>('playBtn'),
    stopBtn: el<HTMLButtonElement>('stopBtn'),
    meterL: el<HTMLElement>('meterL'),
    meterR: el<HTMLElement>('meterR'),
    errorBox: el<HTMLPreElement>('errorBox'),
    midiLed: el<HTMLElement>('midiLed'),
    midiStatus: el<HTMLElement>('midiStatus'),
    midiNotes: el<HTMLElement>('midiNotes'),
    midiPanic: el<HTMLButtonElement>('midiPanic'),
    midiRescan: el<HTMLButtonElement>('midiRescan')
  };
}

export function setStatus(ui: UiElements, text: string): void {
  ui.statusLabel.textContent = text;
}

export function setPlugin(ui: UiElements, label: string): void {
  ui.pluginLabel.textContent = label;
}

export function setSampleRate(ui: UiElements, hz: number | null): void {
  ui.sampleRateLabel.textContent = hz == null ? '—' : `${hz.toFixed(0)} Hz`;
}

export function setCoi(ui: UiElements, isolated: boolean): void {
  ui.coiLabel.textContent = isolated
    ? 'yes (threads available)'
    : 'no (single-thread fallback)';
}

export function setAudioState(
  ui: UiElements,
  state: string,
  extra?: string
): void {
  ui.audioStateLabel.textContent = extra ? `${state} · ${extra}` : state;
}

export function showError(ui: UiElements, err: unknown): void {
  const text = err instanceof Error ? `${err.name}: ${err.message}` : String(err);
  ui.errorBox.textContent = text;
  ui.errorBox.hidden = false;
  console.error('[wclap-host]', err);
}

export function clearError(ui: UiElements): void {
  ui.errorBox.textContent = '';
  ui.errorBox.hidden = true;
}

let midiLedTimer: number | null = null;
export function flashMidiLed(ui: UiElements): void {
  ui.midiLed.classList.add('midiLedOn');
  if (midiLedTimer != null) window.clearTimeout(midiLedTimer);
  midiLedTimer = window.setTimeout(() => {
    ui.midiLed.classList.remove('midiLedOn');
    midiLedTimer = null;
  }, 110);
}

export function setMidiStatus(ui: UiElements, text: string): void {
  ui.midiStatus.textContent = text;
}

export function setMidiNotes(ui: UiElements, text: string): void {
  ui.midiNotes.textContent = text;
}

export function setMeters(ui: UiElements, rmsL: number, rmsR: number): void {
  // -60 dBFS floor, 0 dBFS ceiling for the visual.
  const toPct = (rms: number): number => {
    if (!isFinite(rms) || rms <= 0) return 0;
    const db = 20 * Math.log10(rms);
    return Math.max(0, Math.min(100, ((db + 60) / 60) * 100));
  };
  ui.meterL.style.width = `${toPct(rmsL)}%`;
  ui.meterR.style.width = `${toPct(rmsR)}%`;
}
