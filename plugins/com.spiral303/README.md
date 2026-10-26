# com.spiral303

Plugins by **Spiralfractal** — home-studio audio software, currently one
entry: a TB-303 line.

## Plugins

| Plugin       | Authored in | Notes |
|--------------|-------------|-------|
| `spiral303`  | C++ (WASI-SDK → wasm32) | Roland TB-303 emulation: sequencer, gate wheel, filter with accent/slide, canvas UI |

## Contact

- Source: [github.com/Spiralfractalplugins/spiral303](https://github.com/Spiralfractalplugins/spiral303)
- GitHub org for this vendor folder: [Spiralfractalplugins](https://github.com/Spiralfractalplugins)

## License

The WCLAP bundle in this vendor folder is released under **Apache-2.0**:
the wasm module is built from sources that do not link JUCE (a small
JUCE-shaped shim stands in for it), so the bundle carries no GPL obligation.

The same DSP also ships as a native VST3/AU/LV2/CLAP plugin, and that build
*does* link JUCE under its GPLv3 option — the native plugin is therefore
GPL-3.0-only. Two artifacts, two licences, one author.
