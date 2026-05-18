// Base class for Plinken plugin UI widgets implemented as custom elements.
// Each widget binds to one wclap-host endpoint: annotations arrive in
// onMeta(), live values in onValue(), and UI gestures route back via
// write().
//
// Usage from a plugin's ui/index.html:
//
//   import { PlinkenWidget } from '../widget-lib/widget-base.mjs';
//
//   class MyDial extends PlinkenWidget {
//     onMeta(meta)  { /* render from meta.min/max/init/unit/step/... */ }
//     onValue(v)    { /* update visuals */ }
//   }
//   customElements.define('my-dial', MyDial);
//
//   document.querySelector('my-dial[endpoint="gain"]').setConnection(conn);

export class PlinkenWidget extends HTMLElement {
  #conn = null;
  #ep = null;
  #listener = null;

  setConnection(conn) {
    this.#conn = conn;
    this.#ep = this.getAttribute('endpoint');
    this.#mount();
  }

  async #mount() {
    // Pull annotations once — min/max/init/unit/step/text all come from the patch
    const status = await this.#conn.requestStatusUpdate();
    const meta = status.parameters.find(p => p.endpointID === this.#ep);
    this.onMeta(meta);

    // Subscribe to live changes
    this.#listener = (v) => this.onValue(v);
    this.#conn.addParameterListener(this.#ep, this.#listener);

    // Pull current value
    this.#conn.requestParameterValue(this.#ep);
  }

  disconnectedCallback() {
    if (this.#listener) this.#conn?.removeParameterListener(this.#ep, this.#listener);
  }

  // Subclasses override these:
  onMeta(meta) {}     // got annotations — render initial state
  onValue(v) {}       // got new value — update transform/opacity/text

  // Helper for UI → DSP writes with gesture grouping
  write(value, gesture = false) {
    if (gesture) this.#conn.sendParameterGestureStart(this.#ep);
    this.#conn.sendEventOrValue(this.#ep, value);
    if (gesture) this.#conn.sendParameterGestureEnd(this.#ep);
  }
}
