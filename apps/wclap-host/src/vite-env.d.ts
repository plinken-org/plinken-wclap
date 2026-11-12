/// <reference types="vite/client" />

// Vite emits a bundled worker entry and returns its URL as a string for the
// `?worker&url` import suffix. The same bundled file works as an AudioWorklet
// module too, since both globals support ES modules.
declare module '*?worker&url' {
  const src: string;
  export default src;
}

/** `v<version> · <commit> · <date>` of this build (vite.config.ts). */
declare const __BUILD_LABEL__: string;
