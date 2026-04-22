use crate::{
    apu::{Pulse, Triangle, Noise, Dmc},
    cpu::{Cpu, /* Interrupt */},
    clock::ClockDivider,
};

#[allow(dead_code)]
enum StatusFlag {
    DMCInterrupt    = 0b1000_0000,
    FrameInterrupt  = 0b0100_0000,
    DMC             = 0b0001_0000,
    Noise           = 0b0000_1000,
    Triangle        = 0b0000_0100,
    Square2         = 0b0000_0010,
    Square1         = 0b0000_0001,
}

#[derive(Clone, PartialEq)]
enum FrameCounterMode {
    FourStep    = 0,
    FiveStep    = 1,
}

/// First-order IIR filter chain modelling the NES's analog stages. Run at
/// CPU rate (1.789 MHz) so the LP stage doubles as an anti-aliasing filter
/// before the output sampler decimates to ~48 kHz. Three cascaded LP poles
/// at 14 kHz give an 18 dB/oct rolloff, pushing alias energy well below
/// audibility at the output rate.
///
/// - High-pass: `y = a * (y_prev + x - x_prev)` with `a = exp(-2π fc / fs)`
/// - Low-pass:  `y = (1 - a) * x + a * y_prev`
struct FilterChain {
    hp90_a: f32,
    hp90_xn1: f32,
    hp90_yn1: f32,
    hp440_a: f32,
    hp440_xn1: f32,
    hp440_yn1: f32,
    lp14k_a: f32,
    lp14k_yn1_a: f32,
    lp14k_yn1_b: f32,
    lp14k_yn1_c: f32,
}

impl FilterChain {
    fn new (filter_rate: f32) -> Self {
        let tau = std::f32::consts::TAU;
        Self {
            hp90_a: (-tau * 90.0 / filter_rate).exp(),
            hp90_xn1: 0.0,
            hp90_yn1: 0.0,
            hp440_a: (-tau * 440.0 / filter_rate).exp(),
            hp440_xn1: 0.0,
            hp440_yn1: 0.0,
            lp14k_a: (-tau * 14_000.0 / filter_rate).exp(),
            lp14k_yn1_a: 0.0,
            lp14k_yn1_b: 0.0,
            lp14k_yn1_c: 0.0,
        }
    }

    fn process (&mut self, input: f32) -> f32 {
        // HP at 90 Hz — removes DC and very low rumble
        let hp90 = self.hp90_a * (self.hp90_yn1 + input - self.hp90_xn1);
        self.hp90_xn1 = input;
        self.hp90_yn1 = hp90;

        // HP at 440 Hz — matches the RF demodulator in the Famicom
        let hp440 = self.hp440_a * (self.hp440_yn1 + hp90 - self.hp440_xn1);
        self.hp440_xn1 = hp90;
        self.hp440_yn1 = hp440;

        // 3-stage cascaded LP at 14 kHz — removes aliased harmonics
        let one_minus_a = 1.0 - self.lp14k_a;
        let lp_a = one_minus_a * hp440 + self.lp14k_a * self.lp14k_yn1_a;
        self.lp14k_yn1_a = lp_a;
        let lp_b = one_minus_a * lp_a + self.lp14k_a * self.lp14k_yn1_b;
        self.lp14k_yn1_b = lp_b;
        let lp_c = one_minus_a * lp_b + self.lp14k_a * self.lp14k_yn1_c;
        self.lp14k_yn1_c = lp_c;

        lp_c
    }
}

/**
 * https://wiki.nesdev.org/w/index.php/APU
 */
pub struct Apu {
    status: u8,
    mode: FrameCounterMode,
    irq_inhibit: bool,
    square_1: Pulse,
    square_2: Pulse,
    triangle: Triangle,
    noise: Noise,
    pub dmc: Dmc,
    filters: FilterChain,
    last_sample: f32,
    buffer: Vec<f32>,
    frame: usize,
    /// Per-channel mute flags for debugging: [pulse1, pulse2, triangle, noise, dmc].
    /// Muted channels still run their internal state (length/envelope/sweep)
    /// so enabling them mid-song resumes correctly; only their mix output is
    /// forced to zero.
    muted: [bool; 5],
    pub clock: ClockDivider,
    pub clock_sample: ClockDivider,
}

impl Apu {
    pub fn new (sample_rate: f64) -> Self {
        Self {
            status: 0,
            mode: FrameCounterMode::FiveStep,
            irq_inhibit: false,
            square_1: Pulse::new(1),
            square_2: Pulse::new(2),
            triangle: Triangle::new(),
            noise: Noise::new(),
            dmc: Dmc::new(),
            // Filters run at CPU rate, not output rate, so they can anti-alias.
            filters: FilterChain::new(crate::clock::CLOCK_CPU_NTSC as f32),
            last_sample: 0.0,
            buffer: vec![],
            frame: 0,
            muted: [false; 5],
            clock: ClockDivider::new(crate::clock::CLOCK_CPU_NTSC),
            clock_sample: ClockDivider::new(sample_rate),
        }
    }

    pub fn set_channel_muted (&mut self, channel: usize, muted: bool) {
        if channel < self.muted.len() {
            self.muted[channel] = muted;
        }
    }

    pub fn tick (&mut self, time: f64, cpu: &mut Cpu) {
        if self.clock.tick(time) {
            self.cycle(cpu);
        }

        if self.clock_sample.tick(time) {
            self.sample();
        }
    }

    pub fn cycle (&mut self, cpu: &mut Cpu) {
        // Triangle and DMC are both clocked at CPU rate.
        self.triangle.cycle_timer();
        self.dmc.cycle_timer();

        if self.clock.cycles % 2 == 0 {
            self.square_1.cycle_timer();
            self.square_2.cycle_timer();
            self.noise.cycle_timer();
            self.frame += 1;

            self.cycle_frame(cpu);
        }

        // Filter at CPU rate so every channel transition feeds the
        // anti-aliasing filter before the output decimation. The buffered
        // output sample is whatever last_sample held when the output-rate
        // accumulator fires.
        let mixed = self.mix();
        self.last_sample = self.filters.process(mixed);
    }

    fn tick_quarter_frame (&mut self) {
        self.square_1.cycle_envelope();
        self.square_2.cycle_envelope();
        self.triangle.cycle_linear();
        self.noise.cycle_envelope();
    }

    fn tick_half_frame (&mut self) {
        self.square_1.cycle_length();
        self.square_1.cycle_sweep();
        self.square_2.cycle_length();
        self.square_2.cycle_sweep();
        self.triangle.cycle_length();
        self.noise.cycle_length();
    }

    /**
     * https://wiki.nesdev.org/w/index.php/APU_Frame_Counter
     */
    pub fn cycle_frame (&mut self, _cpu: &mut Cpu) {
        match self.mode {
            FrameCounterMode::FourStep => {
                match self.frame {
                    3729 => {
                        self.tick_quarter_frame();
                    },
                    7457 => {
                        self.tick_quarter_frame();
                        self.tick_half_frame();
                    },
                    11186 => {
                        self.tick_quarter_frame();
                    },
                    14915 => {
                        self.tick_quarter_frame();
                        self.tick_half_frame();
                        if !self.irq_inhibit {
                            self.status |= StatusFlag::FrameInterrupt as u8;
                            // cpu.interrupt_request(Interrupt::IRQ);
                        }
                        self.frame = 0;
                    },
                    _ => {},
                }
            },
            FrameCounterMode::FiveStep => {
                match self.frame {
                    3729 => {
                        self.tick_quarter_frame();
                    },
                    7457 => {
                        self.tick_quarter_frame();
                        self.tick_half_frame();
                    },
                    11186 => {
                        self.tick_quarter_frame();
                    },
                    18641 => {
                        self.tick_quarter_frame();
                        self.tick_half_frame();
                        self.frame = 0;
                    },
                    _ => {},
                }
            },
        }
    }

    pub fn reset (&mut self) {
        self.status = 0;
        self.frame = 0;
        self.write(0x4015, 0);
    }

    /**
     * Two-stage nonlinear NES mixer.
     * https://wiki.nesdev.org/w/index.php/APU_Mixer
     */
    pub fn mix (&self) -> f32 {
        let p1  = if self.muted[0] { 0.0 } else { self.square_1.output() as f32 };
        let p2  = if self.muted[1] { 0.0 } else { self.square_2.output() as f32 };
        let tri = if self.muted[2] { 0.0 } else { self.triangle.output() as f32 };
        let noi = if self.muted[3] { 0.0 } else { self.noise.output() as f32 };
        let dmc = if self.muted[4] { 0.0 } else { self.dmc.output() as f32 };

        let pulse_out = if p1 + p2 > 0.0 {
            95.88 / (8128.0 / (p1 + p2) + 100.0)
        } else {
            0.0
        };

        let tnd_sum = tri / 8227.0 + noi / 12241.0 + dmc / 22638.0;
        let tnd_out = if tnd_sum > 0.0 {
            159.79 / (1.0 / tnd_sum + 100.0)
        } else {
            0.0
        };

        pulse_out + tnd_out
    }

    /// Push the most recent CPU-rate filtered sample to the output buffer.
    /// Called by the emulator at the output sample rate (~48 kHz) — this is
    /// the decimation step; anti-aliasing already happened in `cycle()`.
    pub fn sample (&mut self) {
        self.buffer.push(self.last_sample);
    }

    /// Flush the sound sample buffer and returns its content
    pub fn flush (&mut self) -> Vec<f32> {
        std::mem::take(&mut self.buffer)
    }

    pub fn read (&mut self, address: u16) -> u8 {
        match address {
            // Status
            0x4015 => {
                let status =
                      (if self.square_1.length > 0 { 0x01 } else { 0 })
                    | (if self.square_2.length > 0 { 0x02 } else { 0 })
                    | (if self.triangle.length > 0 { 0x04 } else { 0 })
                    | (if self.noise.length > 0    { 0x08 } else { 0 })
                    | (if self.dmc.active()        { 0x10 } else { 0 })
                    | (if (self.status & StatusFlag::FrameInterrupt as u8) > 0 { 0x40 } else { 0 })
                    | (if self.dmc.irq_flag        { 0x80 } else { 0 });
                // Reading $4015 clears the frame IRQ flag (but not the DMC IRQ).
                self.status &= !(StatusFlag::FrameInterrupt as u8);
                status
            },
            _ => panic!("Invalid APU read @ {:#x}", address),
        }
    }

    pub fn peek (&self, address: u16) -> Option<u8> {
        match address {
            0x4015 => Some(self.status),
            _ => None,
        }
    }

    pub fn write (&mut self, address: u16, data: u8) {
        match address {
            // Pulse 1
            0x4000 => self.square_1.write_ctrl(data),
            0x4001 => self.square_1.write_sweep(data),
            0x4002 => self.square_1.write_lo(data),
            0x4003 => self.square_1.write_hi(data),
            // Pulse 2
            0x4004 => self.square_2.write_ctrl(data),
            0x4005 => self.square_2.write_sweep(data),
            0x4006 => self.square_2.write_lo(data),
            0x4007 => self.square_2.write_hi(data),
            // Triangle
            0x4008 => self.triangle.write_ctrl(data),
            0x4009 => {},
            0x400A => self.triangle.write_lo(data),
            0x400B => self.triangle.write_hi(data),
            // Noise
            0x400C => self.noise.write_ctrl(data),
            0x400D => {},
            0x400E => self.noise.write_mode(data),
            0x400F => self.noise.write_length(data),
            // DMC
            0x4010 => self.dmc.write_ctrl(data),
            0x4011 => self.dmc.write_dac(data),
            0x4012 => self.dmc.write_address(data),
            0x4013 => self.dmc.write_length(data),
            // Status (channel enable)
            0x4015 => {
                if (data & StatusFlag::Square1 as u8) > 0  { self.square_1.enable(); } else { self.square_1.disable(); }
                if (data & StatusFlag::Square2 as u8) > 0  { self.square_2.enable(); } else { self.square_2.disable(); }
                if (data & StatusFlag::Triangle as u8) > 0 { self.triangle.enable(); } else { self.triangle.disable(); }
                if (data & StatusFlag::Noise as u8) > 0    { self.noise.enable(); } else { self.noise.disable(); }
                if (data & StatusFlag::DMC as u8) > 0      { self.dmc.enable(); } else { self.dmc.disable(); }
                // Writing $4015 always clears the DMC IRQ flag.
                self.dmc.clear_irq();
            },
            // Frame counter
            0x4017 => {
                self.mode = if (data & 0b1000_0000) > 0 { FrameCounterMode::FiveStep } else { FrameCounterMode::FourStep };
                self.irq_inhibit = (data & 0b0100_0000) > 0;

                if self.irq_inhibit {
                    self.status &= !(StatusFlag::FrameInterrupt as u8);
                }

                if self.mode == FrameCounterMode::FiveStep {
                    self.tick_quarter_frame();
                    self.tick_half_frame();
                }

                self.frame = 0;
            },
            _ => {}, // panic!("Invalid APU write @ {:#x}", address),
        }
    }
}
