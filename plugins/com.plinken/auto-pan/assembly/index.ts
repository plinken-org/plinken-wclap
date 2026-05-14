// Entry point — re-export the CLAP factory plumbing and register every
// plugin module imported below. as-clap collects everything that calls
// `Clap.registerPlugin()` into the factory.

export * from 'as-clap/clap-entry';

import './auto-pan';
