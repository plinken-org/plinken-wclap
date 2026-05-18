// cmaj-wclap-shim.cpp — single-file CLAP entry shim around a cmaj-generated DSP class.
//
// The build script (`build-cmaj-wclap.sh`) compiles this alongside the
// generated header `cmaj generate --target=cpp` produces. Two pieces of
// configuration arrive via `-D` macros on the clang command line:
//
//   CMAJ_CLASS_NAME       — unquoted symbol of the generated DSP struct
//                           (e.g. Piano, Organ — matches the patch
//                           manifest's `mainProcessor`).
//
//   CMAJ_HEADER_PATH      — string literal path to the generated header
//                           (e.g. "generated/cpp/piano.h"). The build
//                           script passes this also via `-include` so the
//                           class is in scope before the shim is parsed.
//
//   WCLAP_PLUGIN_ID       — reverse-DNS CLAP plugin id, string literal.
//   WCLAP_PLUGIN_NAME     — display name, string literal.
//   WCLAP_PLUGIN_VENDOR   — vendor, string literal.
//   WCLAP_PLUGIN_VERSION  — version triple, string literal.
//   WCLAP_PLUGIN_DESC     — short description, string literal.
//   WCLAP_IS_INSTRUMENT   — 1 for instrument, 0 for effect.
//
// Why this is shaped like a single C++ file rather than a generator:
// CLAP's struct-of-function-pointers ABI is small enough that one
// translation unit covering "entry + factory + a single plugin" is
// ~250 lines. A generator would pay for itself the moment we needed two
// plugins per .wasm, but that's not where we are.
//
// What this shim assumes about the cmaj-generated class:
//   * `static constexpr uint32_t numAudioOutputChannels` — 1 or 2.
//     CLAP's audio buffers are non-interleaved per channel, and cmaj's
//     `Array<float, N>` output is mono float32 / interleaved float32 for
//     stereo — we de-interleave only when channels > 1.
//   * `static constexpr uint32_t numAudioInputChannels`  — 0 for now.
//     (Audio-in plugins will need `setInputFrames` wiring; piano/organ
//      are MIDI-driven so this stays at 0.)
//   * MIDI input endpoint with handle == `EndpointHandles::midiIn`.
//     `addEvent_midiIn(std_midi_Message{ message })` consumes a 24-bit
//     MIDI message packed as `(status << 16) | (data1 << 8) | data2` —
//     verified against cmaj's `std__midi__getChannel0to15` etc.
//   * Output stream handle is `EndpointHandles::out`; `copyOutputFrames`
//     emits float32 samples (4 bytes per frame for mono, 8 for stereo).
//
// Parameters are not wired here yet — piano has zero. The Organ has nine
// drawbars and will need a parameters extension; that comes when we
// fix the Organ.cmajor source and need it.

#include <clap/clap.h>
#include <cstdint>
#include <cstring>
#include <new>

// CMAJ_HEADER_PATH is passed both via -include (so the class type is in
// scope) and as a macro purely for documentation. The class itself is
// referenced via CMAJ_CLASS_NAME.

#ifndef CMAJ_CLASS_NAME
#error "build-cmaj-wclap.sh must define CMAJ_CLASS_NAME (the generated DSP class symbol)"
#endif

#ifndef WCLAP_PLUGIN_ID
#error "build-cmaj-wclap.sh must define WCLAP_PLUGIN_ID"
#endif

// ---- Webview / iframe UI ----------------------------------------------------
//
// Enabled by passing `-DWCLAP_HAS_UI=1` (driven from plugin.json's
// `has_ui`). When on, the shim exposes the draft `clap.webview/3`
// extension so the host loads `ui/index.html` from the bundle in an
// iframe. WCLAP_UI_PATH is the path under the bundle, defaulting to
// `/ui/index.html`. The host stitches `file:<modulePath><WCLAP_UI_PATH>`
// before passing the URI to the in-page service-worker proxy
// (see CLAUDE.md "How a plugin UI iframe is routed").
//
// Iframe→plugin messages travel over `webview.receive`. The widget
// transport speaks a tiny CBOR dialect (see widgets/cbor.mjs):
//
//   text(5) "ready"                                          — UI ready
//   map{ text(3) "set": [u32 id, f64 value] }                — Set
//
// We decode just the `Set` shape and apply it via `apply_param`. The
// `Ready` ping is acknowledged with a no-op for now; if a future
// plugin needs to push initial state back, it can call host_webview.send
// — none of the cmaj-authored plugins do yet, so we skip that wiring.

#ifndef WCLAP_HAS_UI
#  define WCLAP_HAS_UI 0
#endif
#ifndef WCLAP_UI_PATH
#  define WCLAP_UI_PATH "/ui/index.html"
#endif

#define CLAP_EXT_WEBVIEW_V3 "clap.webview/3"

// ---- Plugin descriptor ------------------------------------------------------

namespace {

const char* kFeaturesInstrument[] = {
    CLAP_PLUGIN_FEATURE_INSTRUMENT,
    CLAP_PLUGIN_FEATURE_SYNTHESIZER,
    CLAP_PLUGIN_FEATURE_STEREO,
    nullptr
};
const char* kFeaturesEffect[] = {
    CLAP_PLUGIN_FEATURE_AUDIO_EFFECT,
    CLAP_PLUGIN_FEATURE_STEREO,
    nullptr
};

const clap_plugin_descriptor_t kDescriptor = {
    .clap_version = CLAP_VERSION_INIT,
    .id           = WCLAP_PLUGIN_ID,
    .name         = WCLAP_PLUGIN_NAME,
    .vendor       = WCLAP_PLUGIN_VENDOR,
    .url          = nullptr,
    .manual_url   = nullptr,
    .support_url  = nullptr,
    .version      = WCLAP_PLUGIN_VERSION,
    .description  = WCLAP_PLUGIN_DESC,
    .features     = (WCLAP_IS_INSTRUMENT ? kFeaturesInstrument : kFeaturesEffect)
};

// ---- Parameter metadata -----------------------------------------------------
//
// Every value-typed input endpoint that cmaj declares with `init:` /
// `min:` / `max:` annotations becomes a CLAP parameter. The build
// script extracts this table at compile time from the
// `programDetailsJSON` literal cmaj embeds in the generated header and
// drops it here as a list of struct initialisers.
//
// The CLAP param id we expose to hosts IS the cmaj endpoint handle —
// they're disjoint integers per plugin and saving the mapping
// pointlessly invents a lookup. Handles are stable across cmaj
// regenerations because they're assigned in source-declaration order.

struct ParamInfo {
    uint32_t    handle;
    const char* name;
    float       minValue;
    float       maxValue;
    float       defaultValue;
    float       step;      // 0 → continuous; >0 → stepped (drawbars use 1)
};

static constexpr ParamInfo kParams[] = {
#if __has_include(CMAJ_PARAMS_INC)
#  include CMAJ_PARAMS_INC
#endif
};
static constexpr uint32_t kParamCount = sizeof(kParams) / sizeof(kParams[0]);

// ---- Plugin instance --------------------------------------------------------
//
// One plugin = one cmaj DSP instance + per-CLAP-port glue. The DSP is
// embedded (not heap) so its big static state arrays live in the
// PluginInstance's allocation. We also mirror each parameter's
// current value here because cmaj has no `getValue` getter — the
// CLAP host expects `params.get_value` to return whatever was last set
// (and the default until then), so the shim has to remember.
//
// kParamCount is a compile-time constant; we use kParamCount + 1 so a
// patch with zero params still produces a valid C array type.

struct PluginInstance {
    clap_plugin_t   plugin;
    CMAJ_CLASS_NAME dsp;
    double          sampleRate = 48000.0;
    bool            initialised = false;
    float           paramValues[kParamCount + 1] = {};
};

constexpr uint32_t kOutChannels = CMAJ_CLASS_NAME::numAudioOutputChannels;
constexpr uint32_t kInChannels  = CMAJ_CLASS_NAME::numAudioInputChannels;
constexpr uint32_t kMidiInHandle = static_cast<uint32_t>(CMAJ_CLASS_NAME::EndpointHandles::midiIn);
constexpr uint32_t kAudioOutHandle = static_cast<uint32_t>(CMAJ_CLASS_NAME::EndpointHandles::out);

inline PluginInstance* self(const clap_plugin_t* p) {
    return static_cast<PluginInstance*>(p->plugin_data);
}

// ---- Audio ports extension --------------------------------------------------

uint32_t audio_ports_count(const clap_plugin_t*, bool is_input) {
    if (is_input) return kInChannels > 0 ? 1 : 0;
    return 1;
}

bool audio_ports_get(const clap_plugin_t*, uint32_t index, bool is_input,
                     clap_audio_port_info_t* info) {
    if (is_input) {
        if (kInChannels == 0 || index != 0) return false;
        *info = {};
        info->id            = 1;
        std::strncpy(info->name, "in", CLAP_NAME_SIZE);
        info->flags         = CLAP_AUDIO_PORT_IS_MAIN;
        info->channel_count = kInChannels;
        info->port_type     = (kInChannels == 2 ? CLAP_PORT_STEREO : CLAP_PORT_MONO);
        info->in_place_pair = CLAP_INVALID_ID;
        return true;
    }
    if (index != 0) return false;
    *info = {};
    info->id            = 2;
    std::strncpy(info->name, "out", CLAP_NAME_SIZE);
    info->flags         = CLAP_AUDIO_PORT_IS_MAIN;
    info->channel_count = kOutChannels;
    info->port_type     = (kOutChannels == 2 ? CLAP_PORT_STEREO : CLAP_PORT_MONO);
    info->in_place_pair = CLAP_INVALID_ID;
    return true;
}

const clap_plugin_audio_ports_t kAudioPortsExt = {
    .count = audio_ports_count,
    .get   = audio_ports_get,
};

// ---- Note ports extension ---------------------------------------------------

uint32_t note_ports_count(const clap_plugin_t*, bool is_input) {
    return is_input ? 1 : 0;
}

bool note_ports_get(const clap_plugin_t*, uint32_t index, bool is_input,
                    clap_note_port_info_t* info) {
    if (!is_input || index != 0) return false;
    *info = {};
    info->id                 = 10;
    info->supported_dialects = CLAP_NOTE_DIALECT_MIDI | CLAP_NOTE_DIALECT_CLAP | CLAP_NOTE_DIALECT_MIDI_MPE;
    info->preferred_dialect  = CLAP_NOTE_DIALECT_MIDI;
    std::strncpy(info->name, "notes", CLAP_NAME_SIZE);
    return true;
}

const clap_plugin_note_ports_t kNotePortsExt = {
    .count = note_ports_count,
    .get   = note_ports_get,
};

// ---- Parameters extension ---------------------------------------------------

inline int find_param_index(uint32_t paramId) {
    for (uint32_t i = 0; i < kParamCount; ++i) {
        if (kParams[i].handle == paramId) return int(i);
    }
    return -1;
}

uint32_t params_count(const clap_plugin_t*) { return kParamCount; }

bool params_get_info(const clap_plugin_t*, uint32_t index, clap_param_info_t* info) {
    if (index >= kParamCount) return false;
    const auto& p = kParams[index];
    *info = {};
    info->id            = p.handle;
    info->flags         = CLAP_PARAM_IS_AUTOMATABLE | (p.step > 0.0f ? CLAP_PARAM_IS_STEPPED : 0u);
    info->cookie        = nullptr;
    info->min_value     = p.minValue;
    info->max_value     = p.maxValue;
    info->default_value = p.defaultValue;
    std::strncpy(info->name, p.name, CLAP_NAME_SIZE - 1);
    return true;
}

bool params_get_value(const clap_plugin_t* p, clap_id paramId, double* value) {
    auto& s = *self(p);
    const int idx = find_param_index(paramId);
    if (idx < 0) return false;
    *value = double(s.paramValues[idx]);
    return true;
}

bool params_value_to_text(const clap_plugin_t*, clap_id paramId, double value,
                          char* text, uint32_t capacity) {
    const int idx = find_param_index(paramId);
    if (idx < 0) return false;
    // Stepped params render as integers (drawbars 0..8); continuous as 2dp.
    if (kParams[idx].step > 0.0f) {
        std::snprintf(text, capacity, "%d", int(value + (value >= 0 ? 0.5 : -0.5)));
    } else {
        std::snprintf(text, capacity, "%.2f", value);
    }
    return true;
}

bool params_text_to_value(const clap_plugin_t*, clap_id paramId, const char* text,
                          double* value) {
    if (find_param_index(paramId) < 0) return false;
    char* end = nullptr;
    double v = std::strtod(text, &end);
    if (end == text) return false;
    *value = v;
    return true;
}

inline void apply_param(PluginInstance& s, uint32_t paramId, float value) {
    const int idx = find_param_index(paramId);
    if (idx < 0) return;
    s.paramValues[idx] = value;
    s.dsp.setValue(paramId, &value, 0);
}

void params_flush(const clap_plugin_t* p, const clap_input_events_t* in,
                  const clap_output_events_t*) {
    auto& s = *self(p);
    const uint32_t n = in ? in->size(in) : 0;
    for (uint32_t i = 0; i < n; ++i) {
        const clap_event_header_t* h = in->get(in, i);
        if (h->space_id != CLAP_CORE_EVENT_SPACE_ID) continue;
        if (h->type == CLAP_EVENT_PARAM_VALUE) {
            auto* pv = reinterpret_cast<const clap_event_param_value_t*>(h);
            apply_param(s, pv->param_id, float(pv->value));
        }
    }
}

const clap_plugin_params_t kParamsExt = {
    .count          = params_count,
    .get_info       = params_get_info,
    .get_value      = params_get_value,
    .value_to_text  = params_value_to_text,
    .text_to_value  = params_text_to_value,
    .flush          = params_flush,
};

// ---- Webview extension ------------------------------------------------------

#if WCLAP_HAS_UI

// Layout matches `wclap_plugin_webview` in the host (three function ptrs at
// offsets 0/4/8). We expose all three so the host doesn't have to special-
// case missing slots; resource lookups beyond `/ui/index.html` go through
// the service-worker proxy and need no plugin involvement.
struct WebviewExt {
    int32_t  (*get_uri)(const clap_plugin_t*, char* buf, uint32_t cap);
    bool     (*get_resource)(const clap_plugin_t*, const char* path,
                             char* mime_buf, uint32_t mime_cap, void* ostream);
    bool     (*receive)(const clap_plugin_t*, const void* buf, uint32_t size);
};

// Stashed in entry_init when the host hands us our per-instance bundle
// root (e.g. "/plugin/<hash>"). webview.get_uri concatenates this with
// WCLAP_UI_PATH to form the absolute URI the iframe loads.
const char* g_module_path = "";

int32_t webview_get_uri(const clap_plugin_t*, char* buf, uint32_t cap) {
    constexpr const char kPrefix[] = "file:";
    constexpr uint32_t   kPrefixLen = sizeof(kPrefix) - 1;
    const uint32_t modLen = uint32_t(std::strlen(g_module_path));
    const uint32_t uiLen  = uint32_t(std::strlen(WCLAP_UI_PATH));
    const uint32_t total  = kPrefixLen + modLen + uiLen;
    if (cap == 0) return int32_t(total);
    // Truncate (leaving room for NUL) if cap is too small. Won't happen in
    // practice; the host calls with cap=0 first to size correctly.
    const uint32_t writable = (total < cap - 1) ? total : (cap - 1);
    uint32_t w = 0;
    auto write = [&](const char* src, uint32_t len) {
        const uint32_t take = (writable - w < len) ? (writable - w) : len;
        std::memcpy(buf + w, src, take);
        w += take;
    };
    write(kPrefix, kPrefixLen);
    write(g_module_path, modLen);
    write(WCLAP_UI_PATH, uiLen);
    buf[w] = '\0';
    return int32_t(total);
}

bool webview_get_resource(const clap_plugin_t*, const char*, char*, uint32_t, void*) {
    // The SW proxy serves resources directly from the tarball's file map
    // (see CLAUDE.md "How a plugin UI iframe is routed"). The plugin
    // never has to materialise files itself.
    return false;
}

// Big-endian readers — the CBOR transport spec uses network byte order.
inline uint32_t read_be_u32(const uint8_t* p) {
    return (uint32_t(p[0]) << 24) | (uint32_t(p[1]) << 16)
         | (uint32_t(p[2]) << 8)  |  uint32_t(p[3]);
}
inline uint64_t read_be_u64(const uint8_t* p) {
    return (uint64_t(p[0]) << 56) | (uint64_t(p[1]) << 48)
         | (uint64_t(p[2]) << 40) | (uint64_t(p[3]) << 32)
         | (uint64_t(p[4]) << 24) | (uint64_t(p[5]) << 16)
         | (uint64_t(p[6]) << 8)  |  uint64_t(p[7]);
}

bool webview_receive(const clap_plugin_t* p, const void* buf, uint32_t size) {
    if (!buf) return false;
    const uint8_t* b = static_cast<const uint8_t*>(buf);

    // Ready: text(5) "ready" — 6 bytes. We accept and ignore; if a
    // future plugin needs to push initial state it can call
    // host_webview.send here.
    if (size == 6 && b[0] == 0x65 && b[1]=='r' && b[2]=='e'
        && b[3]=='a' && b[4]=='d' && b[5]=='y') {
        return true;
    }
    // Set: 20-byte fixed layout, big-endian:
    //   a1                       map(1)
    //   63 's' 'e' 't'           text(3) "set"
    //   82                       array(2)
    //   1a uXX uXX uXX uXX       u32 id
    //   fb fXX...x8              f64 value
    if (size == 20 && b[0] == 0xa1 && b[1] == 0x63
        && b[2]=='s' && b[3]=='e' && b[4]=='t'
        && b[5] == 0x82 && b[6] == 0x1a && b[11] == 0xfb) {
        const uint32_t id    = read_be_u32(b + 7);
        uint64_t       bits  = read_be_u64(b + 12);
        double         value;
        std::memcpy(&value, &bits, 8);
        apply_param(*self(p), id, float(value));
        return true;
    }
    return false;
}

const WebviewExt kWebviewExt = {
    .get_uri      = webview_get_uri,
    .get_resource = webview_get_resource,
    .receive      = webview_receive,
};

#endif // WCLAP_HAS_UI

// ---- Event dispatch ---------------------------------------------------------
//
// Pack a 3-byte MIDI message (status, data1, data2) into the int32
// layout cmaj's std::midi helpers expect:
//   bits 16-23 → status
//   bits  8-15 → data1
//   bits  0-7  → data2
// (`std__midi__getChannel0to15` shifts right 16 and masks 0xF, etc.)

inline void send_midi_bytes(CMAJ_CLASS_NAME& dsp, uint8_t status, uint8_t d1, uint8_t d2) {
    typename CMAJ_CLASS_NAME::std_midi_Message msg{};
    msg.message = (int32_t(status) << 16) | (int32_t(d1) << 8) | int32_t(d2);
    dsp.addEvent_midiIn(msg);
}

void dispatch_event(PluginInstance& s, const clap_event_header_t* ev) {
    if (ev->space_id != CLAP_CORE_EVENT_SPACE_ID) return;

    switch (ev->type) {
        case CLAP_EVENT_MIDI: {
            auto* m = reinterpret_cast<const clap_event_midi_t*>(ev);
            send_midi_bytes(s.dsp, m->data[0], m->data[1], m->data[2]);
            break;
        }
        case CLAP_EVENT_NOTE_ON:
        case CLAP_EVENT_NOTE_OFF: {
            auto* n = reinterpret_cast<const clap_event_note_t*>(ev);
            uint8_t status = (ev->type == CLAP_EVENT_NOTE_ON ? 0x90 : 0x80)
                           | uint8_t(n->channel & 0x0F);
            uint8_t velocity = uint8_t(n->velocity * 127.0 + 0.5);
            if (velocity > 127) velocity = 127;
            send_midi_bytes(s.dsp, status, uint8_t(n->key & 0x7F), velocity);
            break;
        }
        case CLAP_EVENT_PARAM_VALUE: {
            auto* pv = reinterpret_cast<const clap_event_param_value_t*>(ev);
            apply_param(s, pv->param_id, float(pv->value));
            break;
        }
        // Note expressions / poly param mods deliberately unhandled —
        // we don't expose per-key parameters and there's nothing
        // sensible to translate them to in the cmaj graph.
        default: break;
    }
}

// ---- Process ----------------------------------------------------------------
//
// One CLAP process call advances cmaj N frames in event-segmented
// sub-blocks. Between events we call dsp.advance() once and pull the
// output, then memcpy or de-interleave into CLAP's channel buffers.

void process_block(PluginInstance& self_,
                   const clap_audio_buffer_t* outBuf,
                   uint32_t frameOffset,
                   uint32_t numFrames) {
    if (numFrames == 0) return;
    self_.dsp.advance(int32_t(numFrames));

    // Output is float32 mono (one channel) or interleaved float32 stereo.
    // copyOutputFrames lays it down contiguously, so for mono we copy
    // straight into channel 0; for stereo we copy into a scratch and
    // de-interleave. cmaj's maxFramesPerBlock is 512 (visible in the
    // generated header) — we never exceed it because we sub-block.
    if (kOutChannels == 1) {
        self_.dsp.copyOutputFrames(kAudioOutHandle,
                                   outBuf->data32[0] + frameOffset,
                                   numFrames);
    } else {
        float interleaved[1024];   // 512 frames × 2 ch max — matches maxFramesPerBlock
        self_.dsp.copyOutputFrames(kAudioOutHandle, interleaved, numFrames);
        float* L = outBuf->data32[0] + frameOffset;
        float* R = outBuf->data32[1] + frameOffset;
        for (uint32_t i = 0; i < numFrames; ++i) {
            L[i] = interleaved[i * 2 + 0];
            R[i] = interleaved[i * 2 + 1];
        }
    }
}

clap_process_status plugin_process(const clap_plugin_t* p, const clap_process_t* proc) {
    auto& s = *self(p);
    if (proc->audio_outputs_count < 1 || proc->frames_count == 0) {
        return CLAP_PROCESS_CONTINUE;
    }
    const clap_audio_buffer_t* outBuf = &proc->audio_outputs[0];

    const clap_input_events_t* evIn = proc->in_events;
    const uint32_t nEvents = evIn ? evIn->size(evIn) : 0;
    const uint32_t totalFrames = proc->frames_count;

    // cmaj's maxFramesPerBlock cap (compile-time constant). Process
    // bigger CLAP blocks by chunking. With the synth example sitting at
    // 512 this is rarely tripped — but explicit > silent corruption.
    constexpr uint32_t kCmajMax = CMAJ_CLASS_NAME::maxFramesPerBlock;

    uint32_t frame = 0;
    uint32_t evIdx = 0;
    while (frame < totalFrames) {
        // Drain events whose time == current frame.
        while (evIdx < nEvents) {
            const clap_event_header_t* h = evIn->get(evIn, evIdx);
            if (h->time > frame) break;
            dispatch_event(s, h);
            evIdx++;
        }

        // Run up to the next event (or end of block, or cmaj cap).
        uint32_t nextEvFrame = (evIdx < nEvents)
            ? evIn->get(evIn, evIdx)->time
            : totalFrames;
        uint32_t chunk = nextEvFrame - frame;
        if (chunk > totalFrames - frame) chunk = totalFrames - frame;
        if (chunk > kCmajMax) chunk = kCmajMax;
        if (chunk == 0) chunk = 1;   // defensive — shouldn't happen with monotonic event times

        process_block(s, outBuf, frame, chunk);
        frame += chunk;
    }
    return CLAP_PROCESS_CONTINUE;
}

// ---- Plugin lifecycle -------------------------------------------------------

bool plugin_init(const clap_plugin_t*)         { return true; }

void plugin_destroy(const clap_plugin_t* p) {
    auto* s = self(p);
    s->~PluginInstance();
    ::operator delete(s);
}

// Push every parameter's default value into both the DSP and the
// shim's mirror cache. Called after `initialise` AND `reset`, because
// cmaj's `reset()` zero-clears value endpoints — without re-seeding,
// the host's habitual activate→reset→process sequence would wipe our
// defaults and the plugin would play silent (e.g. organ with all
// drawbars at 0). setValue(handle, &v, 0) snaps; cmaj clamps frames=0
// to a 1-frame ramp internally.
inline void apply_param_defaults(PluginInstance& s) {
    for (uint32_t i = 0; i < kParamCount; ++i) {
        const float v = kParams[i].defaultValue;
        s.paramValues[i] = v;
        float tmp = v;
        s.dsp.setValue(kParams[i].handle, &tmp, 0);
    }
}

bool plugin_activate(const clap_plugin_t* p, double sampleRate,
                     uint32_t, uint32_t) {
    auto& s = *self(p);
    s.sampleRate = sampleRate;
    s.dsp.initialise(0, sampleRate);
    s.initialised = true;
    apply_param_defaults(s);
    return true;
}

void plugin_deactivate(const clap_plugin_t*) {}
bool plugin_start_processing(const clap_plugin_t*) { return true; }
void plugin_stop_processing(const clap_plugin_t*)  {}

void plugin_reset(const clap_plugin_t* p) {
    auto& s = *self(p);
    if (!s.initialised) return;
    s.dsp.reset();
    apply_param_defaults(s);
}

const void* plugin_get_extension(const clap_plugin_t*, const char* id) {
    if (!std::strcmp(id, CLAP_EXT_AUDIO_PORTS)) return &kAudioPortsExt;
    if (!std::strcmp(id, CLAP_EXT_NOTE_PORTS))  return &kNotePortsExt;
    if (!std::strcmp(id, CLAP_EXT_PARAMS) && kParamCount > 0) return &kParamsExt;
#if WCLAP_HAS_UI
    if (!std::strcmp(id, CLAP_EXT_WEBVIEW_V3)) return &kWebviewExt;
#endif
    return nullptr;
}

void plugin_on_main_thread(const clap_plugin_t*) {}

// ---- Factory ---------------------------------------------------------------

uint32_t factory_get_plugin_count(const clap_plugin_factory_t*) { return 1; }

const clap_plugin_descriptor_t*
factory_get_plugin_descriptor(const clap_plugin_factory_t*, uint32_t index) {
    return index == 0 ? &kDescriptor : nullptr;
}

const clap_plugin_t*
factory_create_plugin(const clap_plugin_factory_t*, const clap_host_t*,
                      const char* plugin_id) {
    if (!plugin_id || std::strcmp(plugin_id, kDescriptor.id) != 0) return nullptr;

    void* mem = ::operator new(sizeof(PluginInstance));
    auto* inst = new (mem) PluginInstance{};
    inst->plugin = {};
    inst->plugin.desc            = &kDescriptor;
    inst->plugin.plugin_data     = inst;
    inst->plugin.init            = plugin_init;
    inst->plugin.destroy         = plugin_destroy;
    inst->plugin.activate        = plugin_activate;
    inst->plugin.deactivate      = plugin_deactivate;
    inst->plugin.start_processing= plugin_start_processing;
    inst->plugin.stop_processing = plugin_stop_processing;
    inst->plugin.reset           = plugin_reset;
    inst->plugin.process         = plugin_process;
    inst->plugin.get_extension   = plugin_get_extension;
    inst->plugin.on_main_thread  = plugin_on_main_thread;
    return &inst->plugin;
}

const clap_plugin_factory_t kFactory = {
    .get_plugin_count      = factory_get_plugin_count,
    .get_plugin_descriptor = factory_get_plugin_descriptor,
    .create_plugin         = factory_create_plugin,
};

// ---- Entry ------------------------------------------------------------------

bool entry_init(const char* path) {
#if WCLAP_HAS_UI
    // The host passes us "/plugin/<hash>" — the per-instance bundle
    // root the SW proxy serves files from. We stash a pointer to it
    // (lifetime-managed by the host) so webview.get_uri can compose
    // the absolute UI URI later.
    g_module_path = path ? path : "";
#else
    (void) path;
#endif
    return true;
}
void entry_deinit() {}

const void* entry_get_factory(const char* factory_id) {
    if (!std::strcmp(factory_id, CLAP_PLUGIN_FACTORY_ID)) return &kFactory;
    return nullptr;
}

}   // namespace

extern "C" {
CLAP_EXPORT const clap_plugin_entry_t clap_entry = {
    .clap_version = CLAP_VERSION_INIT,
    .init         = entry_init,
    .deinit       = entry_deinit,
    .get_factory  = entry_get_factory,
};
}
