<script lang="ts">
  // Unlinked live test for the shared library browser — plinken.org/library.
  // Runs the SAME <LibraryBrowser> component the synth GUIs use, against the
  // public mock catalog. Toggling the product is the "one string that differs"
  // between Synome and Pulze in the real plugins.
  import LibraryBrowser from '$lib/LibraryBrowser.svelte';
  import type { Listing } from '$lib/market/types';

  const PRODUCTS = [
    { id: 'com.plinken.synome', label: 'Synome' },
    { id: 'com.plinken.pulze', label: 'Pulze' },
  ];

  let productId = $state('com.plinken.synome');
  let picked = $state<Listing | null>(null);
</script>

<svelte:head><title>Library browser — test</title></svelte:head>

<div class="wrap">
  <div class="bar">
    <strong>Library browser</strong>
    <span class="hint">mock catalog · same component as the synth GUIs</span>
    <div class="switch">
      {#each PRODUCTS as p}
        <button class:active={productId === p.id} onclick={() => (productId = p.id)}>{p.label}</button>
      {/each}
    </div>
  </div>

  <div class="panel">
    <LibraryBrowser {productId} onLoad={(l) => (picked = l)} />
  </div>

  {#if picked}
    <p class="picked">Loaded: <strong>{picked.title}</strong> — {picked.productId}
      {#if picked.padLayout}· {picked.padLayout.pads * picked.padLayout.banks} pads{/if}</p>
  {/if}
</div>

<style>
  .wrap { max-width: 1000px; margin: 0 auto; padding: 16px; color: #e6e8ec; }
  .bar { display: flex; align-items: center; gap: 12px; margin-bottom: 12px; }
  .hint { color: #8a90a0; font-size: 12px; }
  .switch { margin-left: auto; display: flex; gap: 2px; background: #1f232b; border-radius: 6px; padding: 2px; }
  .switch button { background: none; border: 0; color: #9aa0b0; padding: 5px 12px; border-radius: 4px; cursor: pointer; }
  .switch button.active { background: #3a4152; color: #fff; }
  .panel { height: 70vh; border: 1px solid #2a2e37; border-radius: 10px; overflow: hidden; }
  .picked { margin-top: 12px; color: #b9c1d6; font-size: 13px; }
</style>
