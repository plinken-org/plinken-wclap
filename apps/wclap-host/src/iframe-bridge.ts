// Main-thread bridge between plugin iframes and the chain worklet.
//
// In V1 each AudioWorkletNode owned the iframe ↔ worklet relay. In V2 the
// single worklet handles N plugins, so we track per-slot iframe references
// here and route ArrayBuffer messages by slot.

export interface IframeBridgeOptions {
  /** Worklet message port — used to forward iframe ArrayBuffers to the plugin. */
  port: MessagePort;
  /** Look up the iframe for a given slot, if currently mounted. */
  getIframe: (slot: number) => HTMLIFrameElement | null;
}

export class IframeBridge {
  private readonly opts: IframeBridgeOptions;
  private readonly windowToSlot = new WeakMap<Window, number>();

  constructor(opts: IframeBridgeOptions) {
    this.opts = opts;
    window.addEventListener('message', this.onWindowMessage);
  }

  register(slot: number, iframe: HTMLIFrameElement): void {
    // The iframe's contentWindow is only available after load; capture it
    // eagerly via the load event AND now in case it's already navigated.
    const link = () => {
      const w = iframe.contentWindow;
      if (w) this.windowToSlot.set(w, slot);
    };
    link();
    iframe.addEventListener('load', link);
  }

  /** Plugin → iframe push. Worklet message routed here by slot. */
  forwardToIframe(slot: number, buf: ArrayBuffer): void {
    const iframe = this.opts.getIframe(slot);
    if (!iframe?.contentWindow) return;
    iframe.contentWindow.postMessage(buf, '*');
  }

  destroy(): void {
    window.removeEventListener('message', this.onWindowMessage);
  }

  private onWindowMessage = (e: MessageEvent): void => {
    const src = e.source as Window | null;
    if (!src) return;
    const slot = this.windowToSlot.get(src);
    if (slot == null) return;
    const data = e.data;
    if (data instanceof ArrayBuffer) {
      this.opts.port.postMessage(
        { kind: 'plugin-msg', slot, buf: data },
        [data]
      );
    }
  };
}
