// Auto-Pan — a stereo auto-panner driven by a sine LFO.
//
// Parameters
//   0x1001 Speed   — LFO rate in Hz (0.1 .. 20, default 5).
//   0x1002 Wet/Dry — mix between dry input and panned signal (0 .. 1,
//                    default 1 = full wet).
//
// At wet = 1 the LFO sweeps the signal across the stereo field at the
// configured rate using equal-power panning. At wet = 0 the input passes
// through untouched.

import * as Clap from 'as-clap';
import { CNumPtr } from 'as-clap';

const PARAM_SPEED: u32 = 0x1001;
const PARAM_WET: u32 = 0x1002;

const TWO_PI: f32 = 6.28318530717958647692;
const HALF_PI: f32 = 1.57079632679489661923;
const SQRT2: f32 = 1.41421356237309504880;

class AutoPan extends Clap.Plugin {
  sampleRate: f32 = 48000;
  phase: f32 = 0;

  speedTarget: f32 = 5;
  wetTarget: f32 = 1;

  speedSmoothed: f32 = 5;
  wetSmoothed: f32 = 1;

  constructor(host: Clap.clap_host) {
    super(host);
  }

  pluginActivate(sampleRate: f64, minFrames: u32, maxFrames: u32): bool {
    this.sampleRate = f32(sampleRate);
    this.phase = 0;
    this.speedSmoothed = this.speedTarget;
    this.wetSmoothed = this.wetTarget;
    return true;
  }

  pluginProcess(process: Clap.Process): i32 {
    const audioIn = process.audioInputs[0];
    const audioOut = process.audioOutputs[0];
    const length = process.framesCount;

    this.paramsFlush(process.inEvents, process.outEvents);

    let phase = this.phase;
    let speedSm = this.speedSmoothed;
    let wetSm = this.wetSmoothed;
    const speedTarget = this.speedTarget;
    const wetTarget = this.wetTarget;
    const sr = this.sampleRate;
    // ~20 ms one-pole smoothing so parameter changes don't click.
    const smoothCoeff: f32 = f32(1) / (f32(0.02) * sr);

    const bufLIn = audioIn.data32[0];
    const bufRIn = audioIn.data32[1];
    const bufLOut = audioOut.data32[0];
    const bufROut = audioOut.data32[1];

    for (let i: u32 = 0; i < length; i++) {
      speedSm += (speedTarget - speedSm) * smoothCoeff;
      wetSm += (wetTarget - wetSm) * smoothCoeff;

      phase += TWO_PI * speedSm / sr;
      if (phase >= TWO_PI) phase -= TWO_PI;

      const lfo = Mathf.sin(phase);
      const pos = (lfo + f32(1)) * f32(0.5); // [0, 1]
      const lGain = Mathf.cos(pos * HALF_PI) * SQRT2;
      const rGain = Mathf.sin(pos * HALF_PI) * SQRT2;

      const li = bufLIn[i];
      const ri = bufRIn[i];
      const dry = f32(1) - wetSm;

      bufLOut[i] = li * dry + li * lGain * wetSm;
      bufROut[i] = ri * dry + ri * rGain * wetSm;
    }

    this.phase = phase;
    this.speedSmoothed = speedSm;
    this.wetSmoothed = wetSm;

    return Clap.PROCESS_CONTINUE;
  }

  audioPortsCount(isInput: bool): u32 {
    return 1;
  }
  audioPortsGet(index: u32, isInput: bool, info: Clap.AudioPortInfo): bool {
    if (index > 0) return false;
    info.id = isInput ? 0 : 1;
    info.name = 'main';
    info.channelCount = 2;
    info.portType = 'stereo';
    return true;
  }

  paramsCount(): u32 {
    return 2;
  }
  paramsGetInfo(index: u32, info: Clap.ParamInfo): bool {
    if (index == 0) {
      info.id = PARAM_SPEED;
      info.name = 'Speed';
      info.flags = Clap.PARAM_IS_AUTOMATABLE;
      info.minValue = 0.1;
      info.maxValue = 20.0;
      info.defaultValue = 5.0;
      return true;
    }
    if (index == 1) {
      info.id = PARAM_WET;
      info.name = 'Wet/Dry';
      info.flags = Clap.PARAM_IS_AUTOMATABLE;
      info.minValue = 0.0;
      info.maxValue = 1.0;
      info.defaultValue = 1.0;
      return true;
    }
    return false;
  }
  paramsGetValue(id: Clap.clap_id, value: CNumPtr<f64>): bool {
    if (id == PARAM_SPEED) {
      value[0] = this.speedTarget;
      return true;
    }
    if (id == PARAM_WET) {
      value[0] = this.wetTarget;
      return true;
    }
    return false;
  }
  paramsValueToText(id: Clap.clap_id, value: f64): string | null {
    if (id == PARAM_SPEED) {
      const v = Math.round(value * 100) / 100;
      return `${v} Hz`;
    }
    if (id == PARAM_WET) {
      const v = Math.round(value * 100);
      return `${v} %`;
    }
    return null;
  }
  paramsFlush(
    inputEvents: Clap.InputEvents,
    outputEvents: Clap.OutputEvents
  ): void {
    const count = inputEvents.size();
    for (let i: u32 = 0; i < count; i++) {
      const event = inputEvents.get(i);
      if (!this.handleEvent(event)) {
        outputEvents.tryPush(event);
      }
    }
  }

  handleEvent(event: Clap.clap_event_header): bool {
    if (event._space_id != Clap.CORE_EVENT_SPACE_ID) return false;
    if (event._type == Clap.EVENT_PARAM_VALUE) {
      const v = changetype<Clap.clap_event_param_value>(event);
      if (v._param_id == PARAM_SPEED) {
        this.speedTarget = f32(v._value);
        if (this.hostState) this.hostStateMarkDirty();
        return true;
      }
      if (v._param_id == PARAM_WET) {
        this.wetTarget = f32(v._value);
        if (this.hostState) this.hostStateMarkDirty();
        return true;
      }
    }
    return false;
  }

  stateSave(ostream: Clap.OStream): bool {
    const buffer = new ArrayBuffer(8);
    store<f32>(changetype<usize>(buffer), this.speedTarget);
    store<f32>(changetype<usize>(buffer) + 4, this.wetTarget);
    const wrote = ostream.write(changetype<usize>(buffer), 8);
    return wrote == 8;
  }
  stateLoad(istream: Clap.IStream): bool {
    const buffer = new ArrayBuffer(8);
    const read = istream.read(changetype<usize>(buffer), 8);
    if (read != 8) return false;
    this.speedTarget = load<f32>(changetype<usize>(buffer));
    this.wetTarget = load<f32>(changetype<usize>(buffer) + 4);
    return true;
  }
}

const spec = Clap.registerPlugin<AutoPan>('Auto-Pan', 'com.plinken.auto-pan');
spec.vendor = 'Plinken';
spec.description = 'Stereo auto-panner with sine LFO.';
// Feature tags as listed in plugin-features.h. Audio-effect for the type,
// utility because this is a simple insert effect.
spec.features = ['audio-effect', 'utility'];
