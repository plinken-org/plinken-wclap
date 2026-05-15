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
  /** Slot → iframe element. We resolve `contentWindow` lazily on each
   *  incoming message rather than caching it: when `register()` runs the
   *  iframe may not be in the DOM yet, so its contentWindow is null and
   *  any WeakMap would stay empty. Cross-referencing by element avoids
   *  the timing race. */
  private readonly registered = new Map<number, HTMLIFrameElement>();

  constructor(opts: IframeBridgeOptions) {
    this.opts = opts;
    window.addEventListener('message', this.onWindowMessage);
  }

  register(slot: number, iframe: HTMLIFrameElement): void {
    this.registered.set(slot, iframe);
  }

  unregister(slot: number): void {
    this.registered.delete(slot);
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
    // Resolve slot by comparing source against each registered iframe's
    // contentWindow at message time. Same-origin iframes have a valid
    // contentWindow once they're in the DOM, which is before any inline
    // script in the iframe can fire postMessage.
    let slot: number | null = null;
    for (const [s, iframe] of this.registered) {
      if (iframe.contentWindow === src) {
        slot = s;
        break;
      }
    }
    if (slot == null) return;
    const data = e.data;
    if (data instanceof ArrayBuffer) {
      console.log(`[iframe-bridge] slot ${slot} → plugin: ${data.byteLength}b`);
      this.opts.port.postMessage(
        { kind: 'plugin-msg', slot, buf: data },
        [data]
      );
    } else {
      console.log(`[iframe-bridge] slot ${slot} sent non-ArrayBuffer:`, data);
    }
  };
}
