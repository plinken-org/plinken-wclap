<script lang="ts">
  // Shared, product-agnostic library browser. Total reuse across Plinken
  // plugins: the ONLY required input is `productId` (the NKS PLID / clap id).
  // Supplier, deviceType, facets, artwork, story all come from the returned
  // rows — nothing else is hardcoded per instrument. Mount it as:
  //   <LibraryBrowser productId="com.plinken.synome" onLoad={...} />
  //   <LibraryBrowser productId="com.plinken.pulze"  onLoad={...} />
  import type { Listing, MarketClient, Scope } from './market/types';
  import { MockMarketClient } from './market/client';

  interface Props {
    /** NKS plugin identity — the one string that differs between plugins. */
    productId: string;
    /** Injectable data source. Defaults to the mock catalog; swap for HTTP later. */
    client?: MarketClient;
    scope?: Scope;
    visible?: boolean;
    onClose?: () => void;
    onLoad: (listing: Listing) => void;
  }

  let {
    productId,
    client = new MockMarketClient(),
    scope = 'global',
    visible = true,
    onClose,
    onLoad,
  }: Props = $props();

  const scopes: Scope[] = ['global', 'space', 'mine'];

  let base = $state<Listing[]>([]);
  let loading = $state(false);
  let error = $state('');
  let q = $state('');
  let typeFilter = $state('');
  let modeFilter = $state('');
  let tagFilter = $state('');
  let favoritesOnly = $state(false);
  let selectedId = $state('');
  let favorites = $state<Set<string>>(new Set());

  const favKey = () => `plinken.lib.favs.${productId}`;

  // Load persisted favorites when the instrument changes.
  $effect(() => {
    try {
      const raw = globalThis.localStorage?.getItem(favKey());
      favorites = new Set(raw ? (JSON.parse(raw) as string[]) : []);
    } catch {
      favorites = new Set();
    }
  });

  function toggleFav(id: string, e?: Event) {
    e?.stopPropagation();
    const next = new Set(favorites);
    next.has(id) ? next.delete(id) : next.add(id);
    favorites = next;
    try {
      globalThis.localStorage?.setItem(favKey(), JSON.stringify([...next]));
    } catch { /* private mode / no storage — favorites stay in-memory */ }
  }

  // Reload whenever the instrument, scope, or search text changes.
  $effect(() => {
    const req = { app: 'plinken', productId, scope, q: q.trim() || undefined };
    loading = true;
    error = '';
    client
      .listListings(req)
      .then((rows) => { base = rows; })
      .catch((e) => { error = String(e?.message ?? e); base = []; })
      .finally(() => { loading = false; });
  });

  // Supplier + instrument name come from the data, not from props.
  let supplier = $derived(base[0]?.meta.vendor ?? 'PLINKEN');
  let instrument = $derived(base[0]?.meta.bankchain[0] ?? '');

  // Facet rails derived from the loaded set.
  let availableTypes = $derived(
    [...new Set(base.flatMap((l) => l.meta.types.map((path) => path[0]).filter(Boolean)))].sort(),
  );
  let availableModes = $derived([...new Set(base.flatMap((l) => l.meta.modes))].sort());
  let availableTags = $derived([...new Set(base.flatMap((l) => l.tags))].sort());

  let filtered = $derived(
    base.filter((l) => {
      if (favoritesOnly && !favorites.has(l.id)) return false;
      if (typeFilter && !l.meta.types.some((path) => path.includes(typeFilter))) return false;
      if (modeFilter && !l.meta.modes.includes(modeFilter)) return false;
      if (tagFilter && !l.tags.includes(tagFilter)) return false;
      return true;
    }),
  );

  function clearFilters() {
    q = '';
    typeFilter = '';
    modeFilter = '';
    tagFilter = '';
    favoritesOnly = false;
  }

  function select(l: Listing) {
    selectedId = l.id;
    onLoad(l);
  }

  // Deterministic gradient when a library has no artwork ref (mock has none).
  function artwork(l: Listing): string {
    if (l.artworkRef) return `url("${l.artworkRef}") center/cover`;
    let h = 0;
    for (let i = 0; i < l.title.length; i++) h = (h * 31 + l.title.charCodeAt(i)) >>> 0;
    const a = h % 360;
    const b = (a + 40) % 360;
    return `linear-gradient(135deg, hsl(${a} 55% 32%), hsl(${b} 60% 22%))`;
  }

  function initials(name: string): string {
    return name.split(/\s+/).map((w) => w[0]).slice(0, 2).join('').toUpperCase();
  }
</script>

{#if visible}
  <div class="lib-browser" role="dialog" aria-label="Library browser">
    <header class="lib-head">
      <div class="lib-title">
        <strong>{instrument || 'Libraries'}</strong>
        <span class="lib-supplier">{supplier}</span>
      </div>
      <div class="lib-scope" role="tablist">
        {#each scopes as s}
          <button
            role="tab"
            aria-selected={scope === s}
            class:active={scope === s}
            onclick={() => (scope = s)}
          >{s === 'global' ? 'Global' : s === 'space' ? 'Space' : 'Mine'}</button>
        {/each}
      </div>
      {#if onClose}
        <button class="lib-close" aria-label="Close" onclick={onClose}>×</button>
      {/if}
    </header>

    <div class="lib-filters">
      <input type="search" placeholder="Search libraries…" bind:value={q} />
      {#if availableTypes.length}
        <select bind:value={typeFilter} aria-label="Category">
          <option value="">All categories</option>
          {#each availableTypes as t}<option value={t}>{t}</option>{/each}
        </select>
      {/if}
      {#if availableModes.length}
        <select bind:value={modeFilter} aria-label="Mode">
          <option value="">All modes</option>
          {#each availableModes as m}<option value={m}>{m}</option>{/each}
        </select>
      {/if}
      <button class="lib-fav-toggle" class:active={favoritesOnly} aria-pressed={favoritesOnly} onclick={() => (favoritesOnly = !favoritesOnly)}>★ Favorites</button>
      {#if q || typeFilter || modeFilter || tagFilter || favoritesOnly}
        <button class="lib-clear" onclick={clearFilters}>Clear</button>
      {/if}
      <span class="lib-count">{filtered.length} {filtered.length === 1 ? 'library' : 'libraries'}</span>
    </div>

    {#if availableTags.length}
      <div class="lib-tagbar">
        {#each availableTags as t}
          <button class="lib-tag-chip" class:active={tagFilter === t} onclick={() => (tagFilter = tagFilter === t ? '' : t)}>#{t}</button>
        {/each}
      </div>
    {/if}

    {#if loading}
      <div class="lib-status">Loading…</div>
    {:else if error}
      <div class="lib-status lib-error">{error}</div>
    {:else if filtered.length === 0}
      <div class="lib-status">No libraries found.</div>
    {:else}
      <ul class="lib-grid">
        {#each filtered as l (l.id)}
          <li>
            <button class="lib-card" class:selected={selectedId === l.id} onclick={() => select(l)}>
              <div class="lib-hero" style="background: {artwork(l)};">
                <span class="lib-icon">{l.icon ?? '🎵'}</span>
                <span
                  class="lib-fav"
                  class:on={favorites.has(l.id)}
                  role="button"
                  tabindex="0"
                  aria-label={favorites.has(l.id) ? 'Remove favorite' : 'Add favorite'}
                  onclick={(e) => toggleFav(l.id, e)}
                  onkeydown={(e) => { if (e.key === 'Enter' || e.key === ' ') toggleFav(l.id, e); }}
                >{favorites.has(l.id) ? '★' : '☆'}</span>
                {#if l.license === 'free'}<span class="lib-badge">FREE</span>{/if}
              </div>
              <div class="lib-body">
                <span class="lib-name">{l.title}</span>
                <div class="lib-author">
                  <span class="lib-avatar">{initials(l.authorName)}</span>
                  <span>{l.authorName}</span>
                </div>
                <p class="lib-desc">{l.description}</p>
                {#if l.story}<p class="lib-story">{l.story}</p>{/if}
                <div class="lib-tags">
                  <span class="lib-cat">{l.meta.bankchain[1]}</span>
                  {#each l.meta.modes as m}<span class="lib-mode">{m}</span>{/each}
                </div>
                <div class="lib-meta">
                  {#if l.padLayout}
                    <span>{l.padLayout.pads * l.padLayout.banks} pads ({l.padLayout.pads}×{l.padLayout.banks})</span>
                  {/if}
                  <span>{l.downloads.toLocaleString()} ↓</span>
                </div>
              </div>
            </button>
          </li>
        {/each}
      </ul>
    {/if}
  </div>
{/if}

<style>
  .lib-browser {
    display: flex; flex-direction: column; height: 100%;
    background: #16181d; color: #e6e8ec; font: 12px/1.4 system-ui, sans-serif;
  }
  .lib-head { display: flex; align-items: center; gap: 12px; padding: 8px 12px; border-bottom: 1px solid #2a2e37; }
  .lib-title { display: flex; align-items: baseline; gap: 8px; margin-right: auto; }
  .lib-title strong { font-size: 14px; }
  .lib-supplier { color: #8a90a0; font-size: 10px; letter-spacing: 0.06em; }
  .lib-scope { display: flex; gap: 2px; background: #1f232b; border-radius: 6px; padding: 2px; }
  .lib-scope button { background: none; border: 0; color: #9aa0b0; padding: 4px 10px; border-radius: 4px; cursor: pointer; font-size: 11px; }
  .lib-scope button.active { background: #3a4152; color: #fff; }
  .lib-close { background: none; border: 0; color: #9aa0b0; font-size: 18px; cursor: pointer; }
  .lib-filters { display: flex; align-items: center; gap: 8px; padding: 8px 12px; border-bottom: 1px solid #2a2e37; }
  .lib-filters input[type='search'], .lib-filters select {
    background: #1f232b; border: 1px solid #2a2e37; color: #e6e8ec; border-radius: 5px; padding: 5px 8px; font-size: 12px;
  }
  .lib-filters input[type='search'] { flex: 1; min-width: 100px; }
  .lib-fav-toggle { background: #1f232b; border: 1px solid #2a2e37; color: #9aa0b0; border-radius: 5px; padding: 5px 8px; cursor: pointer; }
  .lib-fav-toggle.active { color: #ffcf4d; border-color: #6a5a1f; }
  .lib-clear { background: none; border: 0; color: #8a90a0; cursor: pointer; }
  .lib-count { margin-left: auto; color: #8a90a0; white-space: nowrap; }
  .lib-tagbar { display: flex; flex-wrap: wrap; gap: 6px; padding: 8px 12px; border-bottom: 1px solid #2a2e37; }
  .lib-tag-chip { background: #1f232b; border: 1px solid #2a2e37; color: #9aa0b0; border-radius: 12px; padding: 2px 10px; font-size: 11px; cursor: pointer; }
  .lib-tag-chip.active { background: #3a4152; color: #fff; border-color: #4a5165; }
  .lib-status { padding: 24px; text-align: center; color: #8a90a0; }
  .lib-error { color: #e5877a; }
  .lib-grid {
    list-style: none; margin: 0; padding: 12px;
    display: grid; grid-template-columns: repeat(auto-fill, minmax(200px, 1fr)); gap: 12px; overflow-y: auto;
  }
  .lib-card {
    display: flex; flex-direction: column; width: 100%; text-align: left; padding: 0;
    background: #1f232b; border: 1px solid #2a2e37; border-radius: 10px; cursor: pointer; color: inherit; overflow: hidden;
  }
  .lib-card:hover { border-color: #3a4152; }
  .lib-card.selected { border-color: #5b8cff; }
  .lib-hero { position: relative; height: 72px; display: flex; align-items: center; justify-content: center; }
  .lib-icon { font-size: 30px; filter: drop-shadow(0 1px 2px rgba(0,0,0,0.5)); }
  .lib-fav { position: absolute; top: 6px; right: 8px; font-size: 16px; color: #d7dae0; cursor: pointer; line-height: 1; }
  .lib-fav.on { color: #ffcf4d; }
  .lib-badge { position: absolute; top: 6px; left: 8px; background: #2e7d5b; color: #fff; font-size: 9px; padding: 1px 5px; border-radius: 3px; }
  .lib-body { display: flex; flex-direction: column; gap: 5px; padding: 10px; }
  .lib-name { font-weight: 600; font-size: 13px; }
  .lib-author { display: flex; align-items: center; gap: 6px; color: #9aa0b0; font-size: 11px; }
  .lib-avatar { width: 18px; height: 18px; border-radius: 50%; background: #3a4152; color: #dfe3ea; font-size: 9px; display: inline-flex; align-items: center; justify-content: center; }
  .lib-desc { margin: 0; color: #b9c1d6; font-size: 11px; }
  .lib-story { margin: 0; color: #7a8090; font-size: 10px; font-style: italic; display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; }
  .lib-tags { display: flex; flex-wrap: wrap; gap: 4px; margin-top: 2px; }
  .lib-cat { background: #2a3140; color: #b9c1d6; font-size: 10px; padding: 1px 6px; border-radius: 3px; }
  .lib-mode { background: #24282f; color: #8a90a0; font-size: 10px; padding: 1px 6px; border-radius: 3px; }
  .lib-meta { display: flex; justify-content: space-between; color: #6f7686; font-size: 10px; margin-top: 2px; }
</style>
