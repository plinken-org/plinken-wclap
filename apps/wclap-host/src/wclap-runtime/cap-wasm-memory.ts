// Upstream `getWclap` allocates `new WebAssembly.Memory({ maximum: 32768
// pages, shared: true })` — 2 GB of virtual address space per plugin load.
// On some browsers/machines that fails outright with "RangeError: Out of
// memory"; on others it succeeds the first time but the reservation
// accumulates across reloads until the browser refuses.
//
// 64 KiB pages × 256 = 16 MiB is well above what the sample bundle plugins
// actually use. Five slots × ~2 memories each × 16 MiB ≈ 160 MiB of virtual
// address space, leaving headroom on constrained mobile browsers where the
// upstream 2 GiB reservation tipped us over. Re-importing this file is a
// no-op.

const MAX_PAGES = 256;

declare global {
  interface MemoryConstructor {
    __plinkenOrgCapped?: boolean;
  }
}

const OriginalMemory = WebAssembly.Memory as unknown as {
  new (descriptor: WebAssembly.MemoryDescriptor): WebAssembly.Memory;
  prototype: WebAssembly.Memory;
  __plinkenOrgCapped?: boolean;
};

if (!OriginalMemory.__plinkenOrgCapped) {
  const Patched = function (
    this: WebAssembly.Memory,
    descriptor: WebAssembly.MemoryDescriptor
  ): WebAssembly.Memory {
    let spec: WebAssembly.MemoryDescriptor = descriptor;
    if (
      spec &&
      typeof (spec as { maximum?: number }).maximum === 'number' &&
      (spec as { maximum: number }).maximum > MAX_PAGES
    ) {
      spec = { ...spec, maximum: MAX_PAGES };
    }
    return new OriginalMemory(spec);
  } as unknown as typeof OriginalMemory;
  Patched.prototype = OriginalMemory.prototype;
  Patched.__plinkenOrgCapped = true;
  (WebAssembly as unknown as { Memory: typeof OriginalMemory }).Memory =
    Patched;
}

export {};
