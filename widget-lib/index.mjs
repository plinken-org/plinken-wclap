// @ts-nocheck
// Barrel for @plinken/widget-lib. Importing this file registers every
// shipped widget's custom element (side-effect imports below) and exposes
// the mount helpers a plugin UI uses to wire placed elements to a
// PatchConnection.
//
// A plugin's ui/index.html typically does:
//
//   import { mountAll } from '../widget-lib/index.mjs';
//   mountAll(document, conn);
//
// The designer page imports the same module so its canvas widgets get
// registered and mounted against a MockConnection.

import { PlinkenWidget } from './widget-base.mjs';

import './knob.mjs';
import './fader.mjs';
import './toggle.mjs';
import './meter.mjs';

export { PlinkenWidget };

// Apply x/y/w/h attributes as absolute positioning on every plinken-*
// element under `root`. Safe to call multiple times.
export function applyLayout(root = document) {
  for (const el of root.querySelectorAll('[endpoint]')) {
    const x = el.getAttribute('x');
    const y = el.getAttribute('y');
    const w = el.getAttribute('w');
    const h = el.getAttribute('h');
    if (x != null || y != null) {
      el.style.position = 'absolute';
      el.style.transform = `translate(${x ?? 0}px, ${y ?? 0}px)`;
    }
    if (w != null) el.style.width = `${w}px`;
    if (h != null) el.style.height = `${h}px`;
  }
}

// Walk the tree and call setConnection on every PlinkenWidget.
export function setConnection(conn, root = document) {
  for (const el of root.querySelectorAll('*')) {
    if (el instanceof PlinkenWidget) el.setConnection(conn);
  }
}

// Convenience: layout + connect in one call.
export function mountAll(root = document, conn = null) {
  applyLayout(root);
  if (conn) setConnection(conn, root);
}
