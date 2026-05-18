<script>
  // @ts-nocheck
  import { onMount } from 'svelte';
  import { MockConnection } from '$lib/mock-connection.mjs';
  import '../../../../../widget-lib/widget-lib.css';

  const PALETTE = [
    { kind: 'knob',   label: 'Knob',   w: 56,  h: 56  },
    { kind: 'fader',  label: 'Fader',  w: 20,  h: 200 },
    { kind: 'toggle', label: 'Toggle', w: 40,  h: 22  },
    { kind: 'meter',  label: 'Meter',  w: 20,  h: 200 },
  ];

  // Order matters: META_ATTRS keys form the cache key for re-mounting widgets
  // when the user edits them in the property panel. x/y/w/h sit in LAYOUT_ATTRS
  // so position/resize changes don't tear down the widget.
  const META_ATTRS  = ['endpoint','label','min','max','init','step','unit','text','scaling','format','accent'];
  const LAYOUT_ATTRS = ['x','y','w','h'];

  let canvasW   = $state(480);
  let canvasH   = $state(320);
  let widgets   = $state([]);
  let selected  = $state(null);  // widget id
  let preview   = $state(false);
  let nextId    = 1;

  let canvasEl;
  let conn;
  const connectedNodes = new WeakSet();

  onMount(async () => {
    // Side-effect import to register custom elements. Dynamic to keep
    // SSR happy — customElements doesn't exist on the server.
    await import('../../../../../widget-lib/index.mjs');
    conn = new MockConnection(canvasEl);
    syncConnection();
  });

  $effect(() => {
    // Re-read widgets on every change so the effect tracks the array.
    widgets;
    if (conn) {
      conn.setRoot(canvasEl);
      // Defer one tick so Svelte has committed any newly-rendered elements.
      queueMicrotask(syncConnection);
    }
  });

  $effect(() => {
    if (!conn) return;
    if (preview) conn.startPreview();
    else conn.stopPreview();
  });

  function syncConnection() {
    if (!canvasEl || !conn) return;
    for (const el of canvasEl.querySelectorAll('[endpoint]')) {
      if (connectedNodes.has(el)) continue;
      connectedNodes.add(el);
      el.setConnection(conn);
    }
  }

  function metaKey(w) {
    return META_ATTRS.map(k => `${k}=${w.attrs[k] ?? ''}`).join('|');
  }

  function metaAttrs(w) {
    const out = {};
    for (const k of META_ATTRS) {
      const v = w.attrs[k];
      if (v != null && v !== '') out[k] = String(v);
    }
    return out;
  }

  function addWidget(kind, x, y) {
    const def = PALETTE.find(p => p.kind === kind);
    const id = `w${nextId++}`;
    const attrs = {
      endpoint: `${kind}${nextId - 1}`,
      label: kind.toUpperCase(),
      x: Math.max(0, Math.round(x - def.w / 2)),
      y: Math.max(0, Math.round(y - def.h / 2)),
      w: def.w,
      h: def.h,
    };
    widgets = [...widgets, { id, kind, attrs }];
    selected = id;
  }

  function deleteSelected() {
    if (!selected) return;
    widgets = widgets.filter(w => w.id !== selected);
    selected = null;
  }

  function updateAttr(w, key, value) {
    w.attrs[key] = value;
    widgets = widgets;
  }

  // ----- Palette drag -----

  let dragGhost = $state(null);  // { kind, x, y } in viewport coords

  function startPaletteDrag(e, kind) {
    e.preventDefault();
    const onMove = (ev) => { dragGhost = { kind, x: ev.clientX, y: ev.clientY }; };
    const onUp = (ev) => {
      window.removeEventListener('pointermove', onMove);
      window.removeEventListener('pointerup', onUp);
      dragGhost = null;
      const rect = canvasEl.getBoundingClientRect();
      const x = ev.clientX - rect.left;
      const y = ev.clientY - rect.top;
      if (x >= 0 && y >= 0 && x <= canvasW && y <= canvasH) {
        addWidget(kind, x, y);
      }
    };
    window.addEventListener('pointermove', onMove);
    window.addEventListener('pointerup', onUp);
    dragGhost = { kind, x: e.clientX, y: e.clientY };
  }

  // ----- Canvas widget interaction -----

  function selectWidget(e, w) {
    e.stopPropagation();
    selected = w.id;
  }

  function startMove(e, w) {
    if (e.target.dataset.handle) return;  // resize handle takes precedence
    e.preventDefault();
    e.stopPropagation();
    selected = w.id;
    const startX = e.clientX;
    const startY = e.clientY;
    const origX = w.attrs.x;
    const origY = w.attrs.y;
    const onMove = (ev) => {
      const nx = clamp(origX + ev.clientX - startX, 0, canvasW - w.attrs.w);
      const ny = clamp(origY + ev.clientY - startY, 0, canvasH - w.attrs.h);
      w.attrs.x = Math.round(nx);
      w.attrs.y = Math.round(ny);
      widgets = widgets;
    };
    const onUp = () => {
      window.removeEventListener('pointermove', onMove);
      window.removeEventListener('pointerup', onUp);
    };
    window.addEventListener('pointermove', onMove);
    window.addEventListener('pointerup', onUp);
  }

  function startResize(e, w, handle) {
    e.preventDefault();
    e.stopPropagation();
    const startX = e.clientX;
    const startY = e.clientY;
    const o = { x: w.attrs.x, y: w.attrs.y, ww: w.attrs.w, hh: w.attrs.h };
    const onMove = (ev) => {
      const dx = ev.clientX - startX;
      const dy = ev.clientY - startY;
      let { x, y, ww, hh } = o;
      if (handle.includes('e')) ww = Math.max(8, o.ww + dx);
      if (handle.includes('s')) hh = Math.max(8, o.hh + dy);
      if (handle.includes('w')) { ww = Math.max(8, o.ww - dx); x = o.x + (o.ww - ww); }
      if (handle.includes('n')) { hh = Math.max(8, o.hh - dy); y = o.y + (o.hh - hh); }
      x = clamp(x, 0, canvasW - ww);
      y = clamp(y, 0, canvasH - hh);
      w.attrs.x = Math.round(x);
      w.attrs.y = Math.round(y);
      w.attrs.w = Math.round(ww);
      w.attrs.h = Math.round(hh);
      widgets = widgets;
    };
    const onUp = () => {
      window.removeEventListener('pointermove', onMove);
      window.removeEventListener('pointerup', onUp);
    };
    window.addEventListener('pointermove', onMove);
    window.addEventListener('pointerup', onUp);
  }

  function onCanvasPointerDown(e) {
    if (e.target === canvasEl) selected = null;
  }

  function onKeyDown(e) {
    if (e.key === 'Escape') selected = null;
    if (e.key === 'Delete' || e.key === 'Backspace') {
      if (selected && !isEditableTarget(e.target)) {
        e.preventDefault();
        deleteSelected();
      }
    }
  }

  function isEditableTarget(t) {
    const tag = t?.tagName?.toLowerCase();
    return tag === 'input' || tag === 'textarea' || tag === 'select' || t?.isContentEditable;
  }

  const clamp = (v, lo, hi) => v < lo ? lo : v > hi ? hi : v;

  // ----- Save / Open -----

  function buildHtml() {
    const lines = [
      '<!doctype html>',
      '<meta charset="utf-8">',
      '<link rel="stylesheet" href="../widget-lib/widget-lib.css">',
      '<script type="module" src="../widget-lib/index.mjs"></' + 'script>',
      '',
      `<div class="plinken-ui" data-w="${canvasW}" data-h="${canvasH}" style="position:relative;width:${canvasW}px;height:${canvasH}px">`,
    ];
    for (const w of widgets) {
      const attrs = [];
      for (const k of [...META_ATTRS, ...LAYOUT_ATTRS]) {
        const v = w.attrs[k];
        if (v != null && v !== '') attrs.push(`${k}="${escapeAttr(String(v))}"`);
      }
      lines.push(`  <plinken-${w.kind} ${attrs.join(' ')}></plinken-${w.kind}>`);
    }
    lines.push('</div>');
    lines.push('');
    lines.push('<script type="module">');
    lines.push("  import { mountAll } from '../widget-lib/index.mjs';");
    lines.push('  mountAll(document);');
    lines.push('</' + 'script>');
    return lines.join('\n');
  }

  function escapeAttr(s) {
    return s.replace(/&/g, '&amp;').replace(/"/g, '&quot;').replace(/</g, '&lt;');
  }

  function save() {
    const blob = new Blob([buildHtml()], { type: 'text/html' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = 'index.html';
    a.click();
    URL.revokeObjectURL(url);
  }

  async function open(e) {
    const file = e.target.files?.[0];
    if (!file) return;
    const text = await file.text();
    const doc = new DOMParser().parseFromString(text, 'text/html');
    const root = doc.querySelector('.plinken-ui');
    if (root) {
      const w = parseInt(root.getAttribute('data-w') || '480', 10);
      const h = parseInt(root.getAttribute('data-h') || '320', 10);
      canvasW = w;
      canvasH = h;
    }
    const els = doc.querySelectorAll('[endpoint]');
    const next = [];
    for (const el of els) {
      const kind = el.tagName.toLowerCase().replace(/^plinken-/, '');
      if (!PALETTE.find(p => p.kind === kind)) continue;
      const attrs = {};
      for (const k of [...META_ATTRS, ...LAYOUT_ATTRS]) {
        const v = el.getAttribute(k);
        if (v != null) attrs[k] = isNaN(+v) || ['endpoint','label','unit','text','scaling','format','accent'].includes(k) ? v : +v;
      }
      next.push({ id: `w${nextId++}`, kind, attrs });
    }
    widgets = next;
    selected = null;
    e.target.value = '';
  }

  let selectedWidget = $derived(widgets.find(w => w.id === selected));
</script>

<svelte:window onkeydown={onKeyDown} />

<svelte:head>
  <title>Designer · Plinken</title>
</svelte:head>

<div class="app">
  <header>
    <h1>Designer</h1>
    <div class="canvas-size">
      <label>W <input type="number" min="160" max="1600" bind:value={canvasW} /></label>
      <label>H <input type="number" min="120" max="1200" bind:value={canvasH} /></label>
    </div>
    <label class="preview">
      <input type="checkbox" bind:checked={preview} />
      Preview
    </label>
    <button onclick={save}>Save</button>
    <label class="open">
      Open
      <input type="file" accept=".html,text/html" onchange={open} />
    </label>
  </header>

  <div class="cols">
    <aside class="palette">
      <h2>Widgets</h2>
      {#each PALETTE as item}
        <button
          class="palette-item"
          onpointerdown={(e) => startPaletteDrag(e, item.kind)}
          onclick={() => addWidget(item.kind, canvasW / 2, canvasH / 2)}
        >
          {item.label}
        </button>
      {/each}
      <p class="hint">Click to add to centre, or drag onto the canvas.</p>
    </aside>

    <section class="canvas-wrap">
      <div
        class="canvas"
        bind:this={canvasEl}
        style:width={`${canvasW}px`}
        style:height={`${canvasH}px`}
        onpointerdown={onCanvasPointerDown}
      >
        {#each widgets as w (w.id)}
          <div
            class="placed"
            class:selected={w.id === selected}
            style:transform={`translate(${w.attrs.x}px, ${w.attrs.y}px)`}
            style:width={`${w.attrs.w}px`}
            style:height={`${w.attrs.h}px`}
            onpointerdown={(e) => startMove(e, w)}
          >
            {#key metaKey(w)}
              <svelte:element this={`plinken-${w.kind}`} {...metaAttrs(w)} style="width:100%;height:100%;display:block;" />
            {/key}
            {#if w.id === selected}
              {#each ['nw','n','ne','e','se','s','sw','w'] as h}
                <div class="handle {h}" data-handle={h} onpointerdown={(e) => startResize(e, w, h)}></div>
              {/each}
            {/if}
          </div>
        {/each}
      </div>
    </section>

    <aside class="properties">
      <h2>Properties</h2>
      {#if selectedWidget}
        {@const w = selectedWidget}
        <div class="kind-tag">plinken-{w.kind}</div>

        <fieldset>
          <legend>Binding</legend>
          <label>endpoint
            <input type="text" value={w.attrs.endpoint ?? ''} oninput={(e) => updateAttr(w, 'endpoint', e.currentTarget.value)} />
          </label>
        </fieldset>

        <fieldset>
          <legend>Layout</legend>
          <div class="row">
            <label>x <input type="number" value={w.attrs.x} oninput={(e) => updateAttr(w, 'x', +e.currentTarget.value)} /></label>
            <label>y <input type="number" value={w.attrs.y} oninput={(e) => updateAttr(w, 'y', +e.currentTarget.value)} /></label>
          </div>
          <div class="row">
            <label>w <input type="number" value={w.attrs.w} oninput={(e) => updateAttr(w, 'w', +e.currentTarget.value)} /></label>
            <label>h <input type="number" value={w.attrs.h} oninput={(e) => updateAttr(w, 'h', +e.currentTarget.value)} /></label>
          </div>
        </fieldset>

        <fieldset>
          <legend>Range</legend>
          <div class="row">
            <label>min <input type="number" value={w.attrs.min ?? ''} oninput={(e) => updateAttr(w, 'min', e.currentTarget.value === '' ? undefined : +e.currentTarget.value)} /></label>
            <label>max <input type="number" value={w.attrs.max ?? ''} oninput={(e) => updateAttr(w, 'max', e.currentTarget.value === '' ? undefined : +e.currentTarget.value)} /></label>
          </div>
          <div class="row">
            <label>init <input type="number" value={w.attrs.init ?? ''} oninput={(e) => updateAttr(w, 'init', e.currentTarget.value === '' ? undefined : +e.currentTarget.value)} /></label>
            <label>step <input type="number" value={w.attrs.step ?? ''} oninput={(e) => updateAttr(w, 'step', e.currentTarget.value === '' ? undefined : +e.currentTarget.value)} /></label>
          </div>
          <label>unit <input type="text" value={w.attrs.unit ?? ''} oninput={(e) => updateAttr(w, 'unit', e.currentTarget.value)} /></label>
          <label>text <input type="text" placeholder="pipe|separated|enum" value={w.attrs.text ?? ''} oninput={(e) => updateAttr(w, 'text', e.currentTarget.value)} /></label>
        </fieldset>

        <fieldset>
          <legend>Display</legend>
          <label>label <input type="text" value={w.attrs.label ?? ''} oninput={(e) => updateAttr(w, 'label', e.currentTarget.value)} /></label>
          <label>scaling
            <select value={w.attrs.scaling ?? 'lin'} onchange={(e) => updateAttr(w, 'scaling', e.currentTarget.value)}>
              <option value="lin">lin</option>
              <option value="log">log</option>
            </select>
          </label>
          <label>format <input type="text" placeholder="{'{v:.1f}'} kHz" value={w.attrs.format ?? ''} oninput={(e) => updateAttr(w, 'format', e.currentTarget.value)} /></label>
          <label>accent <input type="color" value={w.attrs.accent ?? '#925db3'} oninput={(e) => updateAttr(w, 'accent', e.currentTarget.value)} /></label>
        </fieldset>

        <button class="danger" onclick={deleteSelected}>Delete</button>
      {:else}
        <p class="hint">Select a widget to edit its properties.</p>
      {/if}
    </aside>
  </div>

  {#if dragGhost}
    <div class="drag-ghost" style:left={`${dragGhost.x}px`} style:top={`${dragGhost.y}px`}>
      {dragGhost.kind}
    </div>
  {/if}
</div>

<style>
  .app {
    display: flex;
    flex-direction: column;
    height: 100vh;
    color: var(--text);
  }

  header {
    display: flex;
    align-items: center;
    gap: 1rem;
    padding: 0.6rem 1rem;
    background: var(--bg-elev);
    border-bottom: 1px solid var(--border-soft);
  }

  h1 {
    margin: 0;
    font-family: var(--font-display);
    font-size: 1.1rem;
    letter-spacing: 0.06em;
  }

  .canvas-size {
    display: flex;
    gap: 0.5rem;
    margin-left: auto;
  }

  .canvas-size label,
  .preview {
    display: inline-flex;
    align-items: center;
    gap: 0.25rem;
    font-size: 0.85rem;
    color: var(--text-muted);
  }

  .canvas-size input {
    width: 4rem;
  }

  header button,
  header .open {
    background: var(--bg-elev-2);
    border: 1px solid var(--border-soft);
    color: var(--text);
    padding: 0.3rem 0.7rem;
    border-radius: 4px;
    font: inherit;
    cursor: pointer;
  }

  header .open input {
    display: none;
  }

  header button:hover,
  header .open:hover {
    border-color: var(--accent);
  }

  input,
  select {
    background: var(--bg-deep);
    border: 1px solid var(--border-soft);
    color: var(--text);
    padding: 0.2rem 0.4rem;
    border-radius: 3px;
    font: inherit;
  }

  .cols {
    display: grid;
    grid-template-columns: 200px 1fr 280px;
    flex: 1;
    min-height: 0;
  }

  aside {
    background: var(--bg-elev);
    border-right: 1px solid var(--border-soft);
    padding: 0.8rem;
    overflow-y: auto;
  }

  .properties {
    border-right: 0;
    border-left: 1px solid var(--border-soft);
  }

  aside h2 {
    font-family: var(--font-display);
    font-size: 0.7rem;
    letter-spacing: 0.16em;
    text-transform: uppercase;
    color: var(--text-dim);
    margin: 0 0 0.6rem;
  }

  .palette-item {
    display: block;
    width: 100%;
    text-align: left;
    background: var(--bg-elev-2);
    border: 1px solid var(--border-soft);
    color: var(--text);
    padding: 0.5rem 0.7rem;
    border-radius: 4px;
    margin-bottom: 0.4rem;
    cursor: grab;
    font: inherit;
    touch-action: none;
  }

  .palette-item:hover {
    border-color: var(--accent);
  }

  .hint {
    color: var(--text-dim);
    font-size: 0.8rem;
    margin: 0.6rem 0 0;
  }

  .canvas-wrap {
    overflow: auto;
    padding: 2rem;
    background:
      linear-gradient(45deg, var(--bg) 25%, transparent 25%),
      linear-gradient(-45deg, var(--bg) 25%, transparent 25%),
      linear-gradient(45deg, transparent 75%, var(--bg) 75%),
      linear-gradient(-45deg, transparent 75%, var(--bg) 75%),
      var(--bg-deep);
    background-size: 24px 24px;
    background-position: 0 0, 0 12px, 12px -12px, -12px 0;
  }

  .canvas {
    position: relative;
    background: var(--plk-bg, #1a1820);
    border: 1px solid var(--border);
    box-shadow: 0 10px 30px rgba(0, 0, 0, 0.4);
    overflow: hidden;
  }

  .placed {
    position: absolute;
    top: 0;
    left: 0;
    cursor: move;
    touch-action: none;
  }

  .placed.selected {
    outline: 1px solid var(--accent);
    outline-offset: 1px;
  }

  .handle {
    position: absolute;
    width: 8px;
    height: 8px;
    background: var(--accent);
    border: 1px solid var(--bg-deep);
  }

  .handle.nw { top: -4px;    left: -4px;   cursor: nwse-resize; }
  .handle.n  { top: -4px;    left: 50%;    transform: translateX(-50%); cursor: ns-resize; }
  .handle.ne { top: -4px;    right: -4px;  cursor: nesw-resize; }
  .handle.e  { top: 50%;     right: -4px;  transform: translateY(-50%); cursor: ew-resize; }
  .handle.se { bottom: -4px; right: -4px;  cursor: nwse-resize; }
  .handle.s  { bottom: -4px; left: 50%;    transform: translateX(-50%); cursor: ns-resize; }
  .handle.sw { bottom: -4px; left: -4px;   cursor: nesw-resize; }
  .handle.w  { top: 50%;     left: -4px;   transform: translateY(-50%); cursor: ew-resize; }

  .kind-tag {
    font-family: var(--font-mono);
    font-size: 0.8rem;
    color: var(--accent);
    margin-bottom: 0.8rem;
  }

  fieldset {
    border: 1px solid var(--border-soft);
    border-radius: 4px;
    padding: 0.5rem 0.6rem;
    margin: 0 0 0.6rem;
  }

  legend {
    font-family: var(--font-display);
    font-size: 0.65rem;
    letter-spacing: 0.14em;
    text-transform: uppercase;
    color: var(--text-dim);
    padding: 0 0.3rem;
  }

  .properties label {
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
    font-size: 0.78rem;
    color: var(--text-muted);
    margin-bottom: 0.4rem;
  }

  .properties .row {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 0.5rem;
  }

  .danger {
    width: 100%;
    background: transparent;
    border: 1px solid var(--accent-purple);
    color: var(--accent-purple);
    padding: 0.4rem;
    border-radius: 4px;
    cursor: pointer;
    font: inherit;
  }

  .danger:hover {
    background: var(--accent-purple);
    color: white;
  }

  .drag-ghost {
    position: fixed;
    pointer-events: none;
    transform: translate(-50%, -50%);
    background: var(--accent);
    color: var(--bg-deep);
    padding: 0.2rem 0.5rem;
    border-radius: 3px;
    font-family: var(--font-mono);
    font-size: 0.75rem;
    z-index: 1000;
  }
</style>
