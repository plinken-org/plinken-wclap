// Upstream `getWclap` allocates `new WebAssembly.Memory({ maximum: 32768
// pages, shared: true })` — 2 GB of virtual address space per plugin load.
// On some browsers/machines that fails outright with "RangeError: Out of
// memory"; on others it succeeds the first time but the reservation
// accumulates across reloads until the browser refuses.
//
// 32 KiB pages × 1024 = 64 MiB is plenty for any plugin in the sample
// bundle (and far more than typical audio plugins ever touch). Cap the
// maximum globally before the upstream module runs — re-importing this
// file is a no-op.

const MAX_PAGES = 1024;

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
