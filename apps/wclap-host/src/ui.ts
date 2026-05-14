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
  drop: HTMLElement;
  fileInput: HTMLInputElement;
  statusLabel: HTMLElement;
  pluginLabel: HTMLElement;
  sampleRateLabel: HTMLElement;
  coiLabel: HTMLElement;
  playBtn: HTMLButtonElement;
  stopBtn: HTMLButtonElement;
  meterL: HTMLElement;
  meterR: HTMLElement;
  errorBox: HTMLPreElement;
}

export function getElements(): UiElements {
  return {
    drop: el<HTMLElement>('drop'),
    fileInput: el<HTMLInputElement>('fileInput'),
    statusLabel: el<HTMLElement>('statusLabel'),
    pluginLabel: el<HTMLElement>('pluginLabel'),
    sampleRateLabel: el<HTMLElement>('sampleRateLabel'),
    coiLabel: el<HTMLElement>('coiLabel'),
    playBtn: el<HTMLButtonElement>('playBtn'),
    stopBtn: el<HTMLButtonElement>('stopBtn'),
    meterL: el<HTMLElement>('meterL'),
    meterR: el<HTMLElement>('meterR'),
    errorBox: el<HTMLPreElement>('errorBox')
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

/**
 * Wire drag/drop + click-to-pick onto the drop zone. Calls `onFile` with the
 * first dropped/picked file. Returns nothing — the listeners live for the
 * lifetime of the page.
 */
export function wireDropZone(
  ui: UiElements,
  onFile: (file: File) => void
): void {
  const onDragOver = (e: DragEvent): void => {
    e.preventDefault();
    ui.drop.classList.add('is-dragover');
  };
  const onDragLeave = (): void => {
    ui.drop.classList.remove('is-dragover');
  };
  const onDrop = (e: DragEvent): void => {
    e.preventDefault();
    ui.drop.classList.remove('is-dragover');
    const file =
      e.dataTransfer?.items?.[0]?.getAsFile?.() ?? e.dataTransfer?.files?.[0];
    if (file) onFile(file);
  };

  ui.drop.addEventListener('dragover', onDragOver);
  ui.drop.addEventListener('dragleave', onDragLeave);
  ui.drop.addEventListener('drop', onDrop);
  ui.drop.addEventListener('click', () => ui.fileInput.click());
  ui.drop.addEventListener('keydown', (e) => {
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      ui.fileInput.click();
    }
  });
  ui.fileInput.addEventListener('change', () => {
    const file = ui.fileInput.files?.[0];
    if (file) onFile(file);
  });
}
