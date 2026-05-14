// Sidecar type declarations for the vendored Signalsmith ES module
// `clap-audionode.mjs`. Only the subset we actually call is typed.

export interface WclapPluginDescriptor {
  id: string;
  name: string;
  vendor?: string;
  version?: string;
  description?: string;
  features?: string[];
}

export interface WclapPluginInfo {
  id: string;
  name: string;
  descriptor?: WclapPluginDescriptor;
}

export interface ClapEffectAudioNode extends AudioWorkletNode {
  descriptor: WclapPluginDescriptor;
  events: Record<string, (...args: unknown[]) => void>;
  getParams?(): Promise<unknown[]>;
  setParam?(id: unknown, value: number): Promise<{ value: number; text: string }>;
  getParam?(id: unknown): Promise<{ value: number; text: string }>;
  saveState?(): Promise<ArrayBuffer | Uint8Array>;
  loadState?(data: ArrayBuffer | Uint8Array): Promise<void>;
  performance?(): Promise<{ block: number; wasm: number; js: number }>;
  openInterface?: unknown;
  closeInterface?: () => void;
}

export interface ClapAudioNodeOptions {
  url?: string;
  module?: ArrayBuffer | Uint8Array | WebAssembly.Module;
  files?: Record<string, ArrayBuffer>;
  timerWorklet?: boolean;
}

export default class ClapAudioNode {
  constructor(options: string | ClapAudioNodeOptions);
  plugins(): Promise<WclapPluginInfo[]>;
  createNode(
    audioContext: AudioContext,
    pluginId?: string | null,
    nodeOptions?: AudioWorkletNodeOptions
  ): Promise<ClapEffectAudioNode>;
}
