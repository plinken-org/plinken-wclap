<script lang="ts">
  const bars = $state(Array.from({ length: 28 }, (_, i) => i));
</script>

<svelte:head>
  <title>PLINKEN.ORG — open WCLAP host & plugins</title>
  <meta
    name="description"
    content="The open / community side of Plinken. A proof-of-concept WCLAP host and plugin set for the WebCLAP project — CLAP audio plugins compiled to wasm32."
  />
  <meta property="og:title" content="PLINKEN.ORG" />
  <meta
    property="og:description"
    content="An open WCLAP host and plugin PoC for the WebCLAP project."
  />
</svelte:head>

<div class="page">
  <nav>
    <a class="logo" href="/">
      <span class="logoMark" aria-hidden="true">◐</span>
      <span class="logoText">PLINKEN<span class="logoTld">.ORG</span></span>
    </a>
    <div class="navLinks">
      <a href="https://plinken.com" target="_blank" rel="noreferrer noopener"
        >plinken.com</a
      >
      <a href="https://github.com/plinken-org/plinken-wclap" target="_blank" rel="noreferrer noopener">
        GitHub
      </a>
    </div>
  </nav>

  <main>
    <section class="hero">
      <div class="badge">PoC · OPEN SOURCE · WIP</div>

      <h1 class="title">PLINKEN<span class="titleTld">.ORG</span></h1>
      <div class="teaser">OPEN WCLAP HOST &amp; PLUGINS</div>

      <p class="lede">
        The community side of <a href="https://plinken.com" target="_blank" rel="noreferrer noopener">Plinken</a>.
        We're implementing
        <a href="https://github.com/WebCLAP" target="_blank" rel="noreferrer noopener"
          >WebCLAP</a
        >
        in the open — a browser-based host plus a handful of authored plugins
        (vocoder &amp; friends), all running as CLAP modules compiled to
        <code>wasm32</code>.
      </p>

      <div class="actions">
        <a
          class="btn primary"
          href="https://wclap.plinken.org"
          target="_blank"
          rel="noreferrer noopener"
        >
          → TRY THE HOST
        </a>
        <a
          class="btn ghost"
          href="https://github.com/plinken-org/plinken-wclap"
          target="_blank"
          rel="noreferrer noopener"
        >
          → GITHUB
        </a>
        <a
          class="btn ghost"
          href="https://github.com/WebCLAP"
          target="_blank"
          rel="noreferrer noopener"
        >
          → WEBCLAP
        </a>
      </div>

      <div class="bars" aria-hidden="true">
        {#each bars as i (i)}
          <span class="bar" style="--i: {i}"></span>
        {/each}
      </div>
    </section>

    <section class="grid">
      <article class="card">
        <div class="cardTag">01 / HOST</div>
        <h2>Browser host</h2>
        <p>
          A SvelteKit-based WCLAP host wiring up <code>wclap-host-js</code>
          into a minimal but real workflow — load plugins, run audio, expose
          a clean API.
        </p>
      </article>

      <article class="card">
        <div class="cardTag">02 / PLUGINS</div>
        <h2>Authored WCLAPs</h2>
        <p>
          A growing set of plugins — starting with a vocoder — built as
          reference implementations of what a wasm-CLAP plugin looks like
          end-to-end.
        </p>
      </article>

      <article class="card">
        <div class="cardTag">03 / REGISTRY</div>
        <h2>Open catalog</h2>
        <p>
          Every plugin is exposed at one CORS-open endpoint —
          <a href="/shelf.json"><code>plinken.org/shelf.json</code></a>.
          Any host, CLI, or MCP server can fetch the catalog and the
          artifacts without auth.
        </p>
      </article>

      <article class="card">
        <div class="cardTag">04 / OPEN</div>
        <h2>In the open</h2>
        <p>
          Public monorepo, public progress, public discussion. Built alongside
          the commercial product at <a href="https://plinken.com" target="_blank" rel="noreferrer noopener">plinken.com</a>.
        </p>
      </article>
    </section>

    <section class="stack">
      <div class="stackHeader">STACK</div>
      <ul>
        <li><span>HOST</span><span>SvelteKit 5 (runes) · TypeScript · Vite</span></li>
        <li><span>RUNTIME</span><span>wasm32 · WebAudio · AudioWorklet</span></li>
        <li><span>PLUGINS</span><span>C++ / AssemblyScript → WCLAP</span></li>
        <li><span>HOSTING</span><span>Cloudflare Workers</span></li>
        <li><span>UPSTREAM</span><span>WebCLAP · wclap-host-js · wclap-bridge</span></li>
      </ul>
    </section>

    <section class="stack plugins">
      <div class="stackHeader">PLUGINS</div>
      <ul>
        <li><span>auto-pan</span><span>Stereo LFO autopanner — FX</span></li>
        <li><span>spectrum</span><span>FFT spectrum analyser with live readout — FX</span></li>
        <li><span>vocal-limiter</span><span>Brick-wall vocal limiter, peak + GR meters — FX</span></li>
        <li><span>synome</span><span>Mono lead synth — instrument (WIP)</span></li>
        <li><span>organ</span><span>Hammond-style drawbar organ, 9 bars × 8 voices — instrument</span></li>
        <li><span>piano</span><span>Stretched-tuning additive piano with stereo spread — instrument</span></li>
      </ul>
    </section>

    <section class="registry">
      <div class="stackHeader">REGISTRY</div>
      <p class="registryLead">
        Every plugin in this monorepo is exposed at one CORS-open
        endpoint — any host UI, CLI, or upcoming MCP server can fetch the
        catalog and pull the bundled artifacts without auth.
      </p>
      <dl class="endpoints">
        <dt>Catalog</dt>
        <dd><a href="/shelf.json">GET plinken.org/shelf.json</a></dd>
        <dt>Artifact</dt>
        <dd>
          <code>GET plinken.org/wclap/&lt;plugin-id&gt;.wclap.wasm</code>
        </dd>
        <dt>Source</dt>
        <dd>
          <a
            href="https://github.com/plinken-org/plinken-wclap/tree/main/plugins"
            target="_blank"
            rel="noreferrer noopener"
            >github.com/plinken-org/plinken-wclap/plugins</a
          >
        </dd>
      </dl>
      <pre class="snippet"><code
          >$ curl https://plinken.org/shelf.json | jq '.items[].id'</code
        ></pre>
    </section>
  </main>

  <footer>
    <div>PLINKEN.ORG</div>
    <div class="footSep">·</div>
    <div>commercial: <a href="https://plinken.com" target="_blank" rel="noreferrer noopener">plinken.com</a></div>
    <div class="footSep">·</div>
    <div>upstream: <a href="https://github.com/WebCLAP" target="_blank" rel="noreferrer noopener">WebCLAP</a></div>
    <div class="footSep">·</div>
    <div>{new Date().getFullYear()}</div>
  </footer>
</div>

<style>
  .page {
    max-width: 72rem;
    margin: 0 auto;
    padding: 1.5rem clamp(1.25rem, 4vw, 2.5rem) 4rem;
  }

  /* ─── nav ─────────────────────────────────────────── */
  nav {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0.5rem 0 2.5rem;
  }

  .logo {
    display: inline-flex;
    align-items: center;
    gap: 0.6rem;
    color: var(--text);
    font-family: var(--font-display);
    font-weight: 600;
    font-size: 1.05rem;
    letter-spacing: 0.06em;
    border-bottom: none;
  }
  .logo:hover { color: var(--accent); }

  .logoMark {
    color: var(--accent-purple);
    font-size: 1.3rem;
    line-height: 1;
  }
  .logoTld { color: var(--text-dim); }

  .navLinks {
    display: flex;
    gap: 1.5rem;
    font-family: var(--font-mono);
    font-size: 0.85rem;
  }
  .navLinks a {
    color: var(--text-muted);
  }
  .navLinks a:hover {
    color: var(--accent);
  }

  /* ─── hero ────────────────────────────────────────── */
  .hero {
    padding-top: clamp(2rem, 8vh, 5rem);
  }

  .badge {
    display: inline-block;
    padding: 0.3rem 0.8rem;
    margin-bottom: 2.5rem;
    font-family: var(--font-mono);
    font-size: 0.72rem;
    letter-spacing: 0.18em;
    color: var(--accent);
    background: rgba(134, 145, 218, 0.08);
    border: 1px solid var(--accent-deep);
    border-radius: 999px;
  }

  .title {
    font-family: var(--font-display);
    font-weight: 700;
    font-size: clamp(2.75rem, 11vw, 7rem);
    line-height: 0.95;
    letter-spacing: -0.02em;
    margin: 0;
    background: linear-gradient(
      135deg,
      #ffffff 0%,
      #c2c2c2 35%,
      #8691da 75%,
      #925db3 100%
    );
    -webkit-background-clip: text;
    background-clip: text;
    -webkit-text-fill-color: transparent;
  }

  .titleTld {
    -webkit-text-fill-color: var(--text-dim);
    font-weight: 300;
  }

  .teaser {
    font-family: var(--font-display);
    font-weight: 300;
    font-size: clamp(0.85rem, 1.6vw, 1.1rem);
    letter-spacing: 0.32em;
    color: var(--text-muted);
    margin: 1.25rem 0 2.5rem;
  }

  .lede {
    font-family: var(--font-sans);
    font-size: clamp(1.05rem, 1.4vw, 1.2rem);
    color: var(--text-muted);
    max-width: 44rem;
    margin: 0 0 2.5rem;
    line-height: 1.65;
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 0.75rem;
    margin-bottom: 3.5rem;
  }

  .btn {
    display: inline-flex;
    align-items: center;
    padding: 0.85rem 1.3rem;
    border-radius: 6px;
    font-family: var(--font-mono);
    font-size: 0.82rem;
    font-weight: 700;
    letter-spacing: 0.08em;
    border: 1px solid var(--border);
    transition: all 140ms ease;
  }

  .btn.primary {
    background: linear-gradient(135deg, var(--accent-deep), var(--accent-purple));
    color: #fff;
    border-color: var(--accent-deep);
    box-shadow: 0 0 0 0 rgba(146, 93, 179, 0);
  }
  .btn.primary:hover {
    transform: translateY(-1px);
    box-shadow: 0 6px 24px -6px rgba(146, 93, 179, 0.6);
    color: #fff;
    border-color: var(--accent-purple);
  }

  .btn.ghost {
    background: transparent;
    color: var(--text-muted);
  }
  .btn.ghost:hover {
    background: var(--bg-elev);
    color: var(--text);
    border-color: var(--accent-deep);
  }

  /* ─── animated audio bars ─────────────────────────── */
  .bars {
    display: flex;
    gap: 3px;
    align-items: flex-end;
    height: 36px;
    margin: 0;
    opacity: 0.7;
  }
  .bar {
    flex: 1;
    background: linear-gradient(
      to top,
      var(--accent-deep),
      var(--accent-purple)
    );
    border-radius: 1px;
    animation: pulse 1.8s ease-in-out infinite;
    animation-delay: calc(var(--i) * 55ms);
    height: 20%;
  }
  @keyframes pulse {
    0%, 100% { height: 12%; opacity: 0.35; }
    50%      { height: 100%; opacity: 1; }
  }
  @media (prefers-reduced-motion: reduce) {
    .bar { animation: none; height: 50%; }
  }

  /* ─── feature grid ────────────────────────────────── */
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(16rem, 1fr));
    gap: 1rem;
    margin: 5rem 0;
  }

  .card {
    background: var(--bg-elev);
    border: 1px solid var(--border-soft);
    border-radius: 10px;
    padding: 1.5rem 1.5rem 1.5rem;
    transition: border-color 140ms ease, transform 140ms ease;
  }
  .card:hover {
    border-color: var(--accent-deep);
    transform: translateY(-2px);
  }

  .cardTag {
    font-family: var(--font-mono);
    font-size: 0.72rem;
    letter-spacing: 0.18em;
    color: var(--accent-teal);
    margin-bottom: 0.75rem;
  }

  .card h2 {
    margin: 0 0 0.6rem;
    font-family: var(--font-display);
    font-size: 1.15rem;
    font-weight: 500;
    color: var(--text);
    letter-spacing: 0.01em;
  }

  .card p {
    margin: 0;
    font-size: 0.95rem;
    color: var(--text-muted);
    line-height: 1.6;
  }

  /* ─── stack table ─────────────────────────────────── */
  .stack {
    margin: 5rem 0 2rem;
    border-top: 1px solid var(--border-soft);
    padding-top: 2.5rem;
  }
  .stackHeader {
    font-family: var(--font-mono);
    font-size: 0.78rem;
    letter-spacing: 0.24em;
    color: var(--accent-teal);
    margin-bottom: 1.5rem;
  }
  .stack ul {
    list-style: none;
    padding: 0;
    margin: 0;
    display: grid;
    gap: 0.6rem;
  }
  .stack li {
    display: grid;
    grid-template-columns: 9rem 1fr;
    gap: 1rem;
    font-family: var(--font-mono);
    font-size: 0.88rem;
    color: var(--text-muted);
    padding: 0.4rem 0;
    border-bottom: 1px dashed var(--border-soft);
  }
  .stack li span:first-child {
    color: var(--accent);
    letter-spacing: 0.08em;
  }

  /* ─── registry ────────────────────────────────────── */
  .registry {
    margin: 4rem 0 2rem;
    border-top: 1px solid var(--border-soft);
    padding-top: 2.5rem;
  }
  .registryLead {
    font-family: var(--font-sans);
    font-size: 1rem;
    color: var(--text-muted);
    max-width: 44rem;
    margin: 0 0 1.5rem;
    line-height: 1.65;
  }
  .endpoints {
    margin: 0 0 1.5rem;
    font-family: var(--font-mono);
    font-size: 0.88rem;
    display: grid;
    grid-template-columns: 6rem 1fr;
    gap: 0.5rem 1rem;
  }
  .endpoints dt {
    color: var(--accent);
    letter-spacing: 0.08em;
  }
  .endpoints dd {
    margin: 0;
    color: var(--text-muted);
    overflow-wrap: anywhere;
  }
  .endpoints code,
  .endpoints a {
    color: var(--text);
  }
  .endpoints a:hover {
    color: var(--accent);
  }
  .snippet {
    margin: 0;
    padding: 0.85rem 1rem;
    background: var(--bg-elev);
    border: 1px solid var(--border-soft);
    border-radius: 6px;
    font-family: var(--font-mono);
    font-size: 0.82rem;
    color: var(--accent);
    overflow-x: auto;
    white-space: pre;
  }

  /* ─── footer ──────────────────────────────────────── */
  footer {
    margin-top: 4rem;
    padding-top: 2rem;
    border-top: 1px solid var(--border-soft);
    font-family: var(--font-mono);
    font-size: 0.78rem;
    color: var(--text-dim);
    display: flex;
    flex-wrap: wrap;
    gap: 0.6rem;
    align-items: center;
  }
  .footSep { color: var(--border); }

  /* ─── responsive ──────────────────────────────────── */
  @media (max-width: 540px) {
    .navLinks { gap: 1rem; font-size: 0.78rem; }
    .stack li { grid-template-columns: 6rem 1fr; font-size: 0.8rem; }
  }
</style>
