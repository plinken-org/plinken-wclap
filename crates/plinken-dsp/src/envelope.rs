//! ADSR envelope.

/// ADSR Envelope
#[derive(Clone, Copy, Default)]
pub struct Envelope {
    stage: EnvStage,
    level: f32,
    attack: f32,
    decay: f32,
    sustain: f32,
    release: f32,
    sample_rate: f32,
    stage_samples: u32,
    release_start: f32,
}

#[derive(Clone, Copy, Default, PartialEq)]
enum EnvStage {
    #[default]
    Idle,
    Attack,
    Decay,
    Sustain,
    Release,
}

impl Envelope {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            attack: 0.01,
            decay: 0.1,
            sustain: 0.7,
            release: 0.3,
            ..Default::default()
        }
    }

    pub fn set_adsr(&mut self, attack: f32, decay: f32, sustain: f32, release: f32) {
        self.attack = attack.max(0.001);
        self.decay = decay.max(0.001);
        self.sustain = sustain;
        self.release = release.max(0.001);
    }

    pub fn trigger(&mut self) {
        self.stage = EnvStage::Attack;
        self.stage_samples = 0;
    }

    pub fn release(&mut self) {
        if self.stage != EnvStage::Idle {
            self.release_start = self.level;
            self.stage = EnvStage::Release;
            self.stage_samples = 0;
        }
    }

    /// Hard-stop the envelope: back to Idle at zero level (used by
    /// all-notes-off style voice resets).
    pub fn reset(&mut self) {
        self.stage = EnvStage::Idle;
        self.level = 0.0;
        self.stage_samples = 0;
    }

    pub fn is_active(&self) -> bool {
        self.stage != EnvStage::Idle
    }

    pub fn is_releasing(&self) -> bool {
        self.stage == EnvStage::Release
    }

    /// Samples spent in the current stage — voice stealing uses this
    /// to find the longest-releasing voice.
    pub fn stage_samples(&self) -> u32 {
        self.stage_samples
    }

    #[inline]
    pub fn process(&mut self) -> f32 {
        self.stage_samples += 1;
        let stage_time = self.stage_samples as f32 / self.sample_rate;

        match self.stage {
            EnvStage::Idle => 0.0,
            EnvStage::Attack => {
                if self.attack <= 0.001 {
                    self.level = 1.0;
                    self.stage = EnvStage::Decay;
                    self.stage_samples = 0;
                } else {
                    self.level = (stage_time / self.attack).min(1.0);
                    if self.level >= 1.0 {
                        self.stage = EnvStage::Decay;
                        self.stage_samples = 0;
                    }
                }
                self.level
            }
            EnvStage::Decay => {
                if self.decay <= 0.001 {
                    self.level = self.sustain;
                    self.stage = EnvStage::Sustain;
                    self.stage_samples = 0;
                } else {
                    let progress = (stage_time / self.decay).min(1.0);
                    self.level = 1.0 - (1.0 - self.sustain) * progress;
                    if progress >= 1.0 {
                        self.stage = EnvStage::Sustain;
                        self.stage_samples = 0;
                    }
                }
                self.level
            }
            EnvStage::Sustain => {
                self.level = self.sustain;
                self.sustain
            }
            EnvStage::Release => {
                if self.release <= 0.001 {
                    self.level = 0.0;
                    self.stage = EnvStage::Idle;
                } else {
                    let progress = (stage_time / self.release).min(1.0);
                    self.level = self.release_start * (1.0 - progress);
                    if self.level <= 0.001 {
                        self.level = 0.0;
                        self.stage = EnvStage::Idle;
                    }
                }
                self.level
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adsr_full_cycle() {
        let sr = 48000.0;
        let mut env = Envelope::new(sr);
        env.set_adsr(0.01, 0.01, 0.5, 0.01);
        assert!(!env.is_active());

        env.trigger();
        assert!(env.is_active());

        // Run through attack: must reach 1.0 within ~attack seconds.
        let mut peak = 0.0f32;
        for _ in 0..(sr * 0.02) as usize {
            peak = peak.max(env.process());
        }
        assert!((peak - 1.0).abs() < 1e-3, "peak={peak}");

        // Settle into sustain.
        let mut level = 0.0;
        for _ in 0..(sr * 0.02) as usize {
            level = env.process();
        }
        assert!((level - 0.5).abs() < 1e-3, "sustain={level}");

        env.release();
        assert!(env.is_releasing());
        for _ in 0..(sr * 0.02) as usize {
            env.process();
        }
        assert!(!env.is_active());
    }

    #[test]
    fn reset_goes_idle() {
        let mut env = Envelope::new(48000.0);
        env.trigger();
        env.process();
        env.reset();
        assert!(!env.is_active());
        assert_eq!(env.process(), 0.0);
    }
}
