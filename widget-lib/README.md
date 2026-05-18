# @plinken/widget-lib

Custom-element widget library for Plinken plugin UIs. Each widget binds
to one wclap-host endpoint and extends `PlinkenWidget` to get annotation
fetch, live value subscription, and UI-gesture write helpers for free.

Distinct from the imperative `widgets/` directory at the repo root,
which stays in place for `vocal-limiter`. New plugin UIs (organ, piano,
auto-panner, …) build on this library.

## Base class

`widget-base.mjs` exports `PlinkenWidget`. Subclass it, override
`onMeta(meta)` and `onValue(v)`, and call `write(value, gesture)` on
user input:

```js
import { PlinkenWidget } from '../widget-lib/widget-base.mjs';

class MyDial extends PlinkenWidget {
  onMeta(meta) { /* render from meta.min/max/init/unit/step/... */ }
  onValue(v)   { /* update visuals */ }
}
customElements.define('my-dial', MyDial);

document.querySelector('my-dial[endpoint="gain"]').setConnection(conn);
```

## Planned widgets

| Widget     | Notes                                                            |
|------------|------------------------------------------------------------------|
| background | the chrome SVG                                                   |
| button     | momentary + toggle                                               |
| dropdown   | discrete enum selection                                          |
| fader      | vertical/horizontal, with stepped variant for organ drawbars     |
| keyboard   | 88-key for piano, configurable range — MIDI in                   |
| knob       | gain/cutoff/Q/freq across most plugins                           |
| label      | static + param-bound with format                                 |
| led        | boolean + optional pulse (taps.gate_active etc.)                 |
| meter      | generalized level meter — peak, RMS, GR (vocal-limiter)          |
| NState     | —                                                                |
| spectrum   | FFT display — for spectrum analyzer + as content slot            |
| switch     | multi-state (auto-pan waveform select)                           |
| Tabs       | —                                                                |
| Toggle     | —                                                                |
| waveform   | oscilloscope / static-display                                    |
| xy-pad     | two-axis control                                                 |
