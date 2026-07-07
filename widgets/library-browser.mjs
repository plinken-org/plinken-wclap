// In-plugin library / patch browser (mock catalog). Vanilla ES module so it
// drops into any plugin's ui/index.html the same way the other widgets do.
// Bundled into every plugin-with-UI tarball by scripts/bundle-wclap.mjs.
//
//   import { mountLibraryBrowser } from '../widgets/library-browser.mjs';
//   const close = mountLibraryBrowser(document.body, {
//     productId: 'com.plinken.synome',
//     onLoad: (listing) => { ...apply... },
//   });
//
// The catalog below is MOCK data (the shape the market service will return).
// Only `productId` differs between Synome and Pulze — same component either way.

const CATALOG = [
  // Pulze — MPC-style drum kits (16×4 pads).
  pulze('Neon Knock', '🥁', 'Punchy neon-lit kit for modern beats.', 'Kits', ['Drums','Kit'], ['One-Shot'], ['punchy','modern','club'], 1280),
  pulze('Dust Circuit', '📼', 'Dusty lo-fi drums with circuit-bent grit.', 'Lo-Fi', ['Drums','Lo-Fi'], ['One-Shot'], ['lofi','dusty','vintage'], 940),
  pulze('Metro Thump', '💥', 'Booming trap 808s and tight snares.', 'Trap', ['Drums','Trap'], ['One-Shot'], ['808','trap','sub'], 2110),
  pulze('Velvet Machine', '🎛️', 'Smooth electronic drum machine tones.', 'Electronic', ['Drums','Electronic'], ['One-Shot'], ['electronic','smooth','analog'], 760),
  pulze('Chrome Breaks', '🔗', 'Chopped breakbeat loops with chrome sheen.', 'Breakbeat', ['Drums','Breakbeat'], ['Loop'], ['breaks','chopped','jungle'], 1495),
  pulze('Basement 808', '🏚️', 'Sub-heavy hip-hop kit from the basement.', 'Hip-Hop', ['Drums','Hip-Hop'], ['One-Shot'], ['808','hiphop','sub'], 1830),
  pulze('Tape Sparks', '✨', 'Tape-saturated percussion with hiss and sparkle.', 'Lo-Fi', ['Drums','Lo-Fi'], ['One-Shot'], ['tape','lofi','percussion'], 615),
  pulze('Midnight Claps', '👏', 'Late-night claps and percussion one-shots.', 'Percussion', ['Drums','Percussion'], ['One-Shot'], ['claps','percussion','night'], 1040),
  // Synome — synth libraries.
  synome('Synome Drift', '🌫️', 'Evolving pads that drift and breathe.', 'Pad', ['Synth','Pad'], ['Poly'], ['pad','cinematic','evolving'], 1370),
  synome('Synome Pulse', '🔷', 'Rhythmic arpeggiated sequences.', 'Sequence', ['Synth','Sequence'], ['Arpeggiated'], ['arp','sequence','rhythmic'], 1560),
  synome('Synome Velvet', '🎹', 'Warm velvet keys and electric pianos.', 'Keys', ['Synth','Keys'], ['Poly'], ['keys','warm','electric'], 880),
  synome('Synome Halo', '😇', 'Ambient washes and cinematic textures.', 'Ambient', ['Synth','Ambient'], ['Poly'], ['ambient','texture','score'], 1210),
  synome('Synome Circuit', '⚡', 'Gritty mono basses with analog bite.', 'Bass', ['Synth','Bass'], ['Mono'], ['bass','mono','analog'], 1940),
  synome('Synome Glass', '🔮', 'Crystalline plucks and glassy bells.', 'Pluck', ['Synth','Pluck'], ['Poly'], ['pluck','bell','bright'], 990),
  synome('Synome Bloom', '🌸', 'Expressive leads that bloom and cut.', 'Lead', ['Synth','Lead'], ['Mono'], ['lead','expressive','solo'], 1420),
  synome('Synome Static', '📺', 'Noise beds and FX one-shots.', 'FX', ['Synth','FX'], ['One-Shot'], ['fx','noise','riser'], 505),
];

function base(productId, instrument, title, icon, description, category, types, modes, tags, downloads) {
  return { productId, title, icon, description, category, types: [types], modes, tags, downloads,
           vendor: 'PLINKEN', author: 'PLINKEN', bankchain: [instrument, category, ''] };
}
function pulze(t, i, d, c, ty, m, tags, dl) {
  return { ...base('com.plinken.pulze', 'Pulze', t, i, d, c, ty, m, tags, dl), padLayout: { pads: 16, banks: 4 } };
}
function synome(t, i, d, c, ty, m, tags, dl) {
  return base('com.plinken.synome', 'Synome', t, i, d, c, ty, m, tags, dl);
}

const CSS = `
.lbx { position:absolute; inset:0; z-index:50; display:flex; flex-direction:column;
  background:var(--bg,#16181d); color:var(--text,#e6e8ec); font:12px/1.4 system-ui,sans-serif; }
.lbx-head { display:flex; align-items:center; gap:10px; padding:8px 12px; border-bottom:1px solid var(--border-soft,#2a2e37); }
.lbx-title { font-size:14px; font-weight:600; margin-right:auto; }
.lbx-supplier { color:var(--text-dim,#8a90a0); font-size:10px; letter-spacing:.06em; }
.lbx-close { background:none; border:0; color:var(--text-dim,#9aa0b0); font-size:20px; cursor:pointer; line-height:1; }
.lbx-filters { display:flex; align-items:center; gap:8px; padding:8px 12px; border-bottom:1px solid var(--border-soft,#2a2e37); flex-wrap:wrap; }
.lbx-filters input, .lbx-filters select { background:var(--bg-soft,#1f232b); border:1px solid var(--border-soft,#2a2e37); color:inherit; border-radius:5px; padding:5px 8px; font-size:12px; }
.lbx-filters input { flex:1; min-width:90px; }
.lbx-fav { background:var(--bg-soft,#1f232b); border:1px solid var(--border-soft,#2a2e37); color:var(--text-dim,#9aa0b0); border-radius:5px; padding:5px 8px; cursor:pointer; }
.lbx-fav.on { color:#ffcf4d; border-color:#6a5a1f; }
.lbx-count { margin-left:auto; color:var(--text-dim,#8a90a0); white-space:nowrap; }
.lbx-grid { list-style:none; margin:0; padding:12px; display:grid; grid-template-columns:repeat(auto-fill,minmax(180px,1fr)); gap:12px; overflow-y:auto; }
.lbx-card { display:flex; flex-direction:column; width:100%; text-align:left; padding:0; background:var(--bg-soft,#1f232b); border:1px solid var(--border-soft,#2a2e37); border-radius:10px; cursor:pointer; color:inherit; overflow:hidden; }
.lbx-card:hover { border-color:#3a4152; }
.lbx-hero { position:relative; height:64px; display:flex; align-items:center; justify-content:center; }
.lbx-icon { font-size:28px; filter:drop-shadow(0 1px 2px rgba(0,0,0,.5)); }
.lbx-star { position:absolute; top:6px; right:8px; font-size:16px; color:#d7dae0; cursor:pointer; }
.lbx-star.on { color:#ffcf4d; }
.lbx-badge { position:absolute; top:6px; left:8px; background:#2e7d5b; color:#fff; font-size:9px; padding:1px 5px; border-radius:3px; }
.lbx-body { display:flex; flex-direction:column; gap:5px; padding:10px; }
.lbx-name { font-weight:600; font-size:13px; }
.lbx-desc { margin:0; color:#b9c1d6; font-size:11px; }
.lbx-story { margin:0; color:#7a8090; font-size:10px; font-style:italic; }
.lbx-tags { display:flex; flex-wrap:wrap; gap:4px; }
.lbx-cat { background:#2a3140; color:#b9c1d6; font-size:10px; padding:1px 6px; border-radius:3px; }
.lbx-mode { background:#24282f; color:#8a90a0; font-size:10px; padding:1px 6px; border-radius:3px; }
.lbx-meta { display:flex; justify-content:space-between; color:#6f7686; font-size:10px; }
`;

function el(tag, cls, txt) { const e = document.createElement(tag); if (cls) e.className = cls; if (txt != null) e.textContent = txt; return e; }

export function mountLibraryBrowser(host, { productId, onLoad, onClose } = {}) {
  if (!document.getElementById('lbx-style')) {
    const s = el('style'); s.id = 'lbx-style'; s.textContent = CSS; document.head.appendChild(s);
  }
  const items = CATALOG.filter((l) => l.productId === productId);
  const supplier = items[0]?.vendor ?? 'PLINKEN';
  const instrument = items[0]?.bankchain[0] ?? 'Libraries';
  const favKey = `plinken.lib.favs.${productId}`;
  let favs = new Set();
  try { favs = new Set(JSON.parse(localStorage.getItem(favKey) || '[]')); } catch {}

  let q = '', typeF = '', modeF = '', favOnly = false;
  const types = [...new Set(items.flatMap((l) => l.types.map((p) => p[0])))].sort();
  const modes = [...new Set(items.flatMap((l) => l.modes))].sort();

  const root = el('div', 'lbx');
  const head = el('div', 'lbx-head');
  const title = el('div', 'lbx-title', instrument);
  head.append(title, Object.assign(el('span', 'lbx-supplier', supplier), {}));
  const closeBtn = el('button', 'lbx-close', '×');
  closeBtn.setAttribute('aria-label', 'Close');
  closeBtn.onclick = () => close();
  head.append(closeBtn);

  const filters = el('div', 'lbx-filters');
  const search = el('input'); search.type = 'search'; search.placeholder = 'Search libraries…';
  const typeSel = el('select'); typeSel.innerHTML = '<option value="">All categories</option>' + types.map((t) => `<option>${t}</option>`).join('');
  const modeSel = el('select'); modeSel.innerHTML = '<option value="">All modes</option>' + modes.map((m) => `<option>${m}</option>`).join('');
  const favBtn = el('button', 'lbx-fav', '★ Favorites');
  const count = el('span', 'lbx-count', '');
  filters.append(search, typeSel, modeSel, favBtn, count);

  const grid = el('ul', 'lbx-grid');
  root.append(head, filters, grid);
  host.appendChild(root);

  function saveFavs() { try { localStorage.setItem(favKey, JSON.stringify([...favs])); } catch {} }

  function render() {
    const ql = q.trim().toLowerCase();
    const shown = items.filter((l) => {
      if (favOnly && !favs.has(l.title)) return false;
      if (typeF && !l.types.some((p) => p.includes(typeF))) return false;
      if (modeF && !l.modes.includes(modeF)) return false;
      if (ql && !`${l.title} ${l.description} ${l.author}`.toLowerCase().includes(ql)) return false;
      return true;
    });
    count.textContent = `${shown.length} ${shown.length === 1 ? 'library' : 'libraries'}`;
    grid.innerHTML = '';
    for (const l of shown) {
      const li = el('li');
      const card = el('button', 'lbx-card');
      const hero = el('div', 'lbx-hero');
      hero.style.background = gradient(l.title);
      hero.append(el('span', 'lbx-icon', l.icon || '🎵'));
      const star = el('span', 'lbx-star' + (favs.has(l.title) ? ' on' : ''), favs.has(l.title) ? '★' : '☆');
      star.onclick = (e) => { e.stopPropagation(); favs.has(l.title) ? favs.delete(l.title) : favs.add(l.title); saveFavs(); render(); };
      hero.append(star, el('span', 'lbx-badge', 'FREE'));
      const body = el('div', 'lbx-body');
      body.append(el('span', 'lbx-name', l.title));
      body.append(Object.assign(el('p', 'lbx-desc', l.description), {}));
      if (l.story) body.append(el('p', 'lbx-story', l.story));
      const tags = el('div', 'lbx-tags');
      tags.append(el('span', 'lbx-cat', l.bankchain[1]));
      for (const m of l.modes) tags.append(el('span', 'lbx-mode', m));
      body.append(tags);
      const meta = el('div', 'lbx-meta');
      meta.append(el('span', null, l.padLayout ? `${l.padLayout.pads * l.padLayout.banks} pads` : ''));
      meta.append(el('span', null, `${l.downloads.toLocaleString()} ↓`));
      body.append(meta);
      card.append(hero, body);
      card.onclick = () => { onLoad && onLoad(l); close(); };
      li.append(card); grid.append(li);
    }
  }

  search.oninput = () => { q = search.value; render(); };
  typeSel.onchange = () => { typeF = typeSel.value; render(); };
  modeSel.onchange = () => { modeF = modeSel.value; render(); };
  favBtn.onclick = () => { favOnly = !favOnly; favBtn.classList.toggle('on', favOnly); render(); };

  function close() { root.remove(); onClose && onClose(); }
  render();
  return close;
}

function gradient(title) {
  let h = 0;
  for (let i = 0; i < title.length; i++) h = (h * 31 + title.charCodeAt(i)) >>> 0;
  const a = h % 360, b = (a + 40) % 360;
  return `linear-gradient(135deg, hsl(${a} 55% 32%), hsl(${b} 60% 22%))`;
}
