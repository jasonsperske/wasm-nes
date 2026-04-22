/// 8-step duty sequences. Each bit controls the output during one step of
/// the pulse sequencer (bit N = output at step N). Stored LSB-first so that
/// `(PULSE_TABLE[duty] >> step) & 1` yields the right level.
const PULSE_TABLE: [u8; 4] = [
    0b0000_0001, // 12.5%   (1 high / 8)
    0b0000_0011, // 25%     (2 high / 8)
    0b0000_1111, // 50%     (4 high / 8)
    0b1111_1100, // 25% inv (6 high / 8, i.e. 75% when inverted)
];

/**
 * https://wiki.nesdev.org/w/index.php/APU_Pulse
 *
 * The sequencer is an 8-step position counter — NOT a rotating bit pattern.
 * Separating `duty` (which pattern) from `step` (which position) is crucial:
 * rewriting `$4000`/`$4004` mid-note must change the level without restarting
 * the waveform. A position-based counter also makes the sweep/envelope glitch-
 * free when a game adjusts volume or duty during a sustained note.
 *
 * The sequencer is reset to step 0 only on `$4003`/`$4007` writes, per nesdev.
 */
#[derive(Clone)]
pub struct Pulse {
    id: u8,
    duty: u8,         // 0..=3
    step: u8,         // 0..=7 (8-step sequencer position)
    pub length: u8,
    length_enabled: bool,
    timer: u16,
    timer_reload: u16,
    volume: u8,
    volume_constant: u8,
    envelope_start: bool,
    envelope_timer: u8,
    envelope_timer_reload: u8,
    envelope_loop: bool,
    envelope_enabled: bool,
    sweep_timer: u8,
    sweep_negate: bool,
    sweep_reload: u8,
    sweep_shift: u8,
    sweep_enabled: bool,
    sweep_reload_flag: bool,
}

impl Pulse {
    pub fn new (id: u8) -> Self {
        Self {
            id,
            duty: 0,
            step: 0,
            length: 0,
            length_enabled: true,
            timer: 0,
            timer_reload: 0,
            volume: 0,
            volume_constant: 15,
            envelope_start: false,
            envelope_timer: 0,
            envelope_timer_reload: 0,
            envelope_loop: false,
            envelope_enabled: true,
            sweep_timer: 0,
            sweep_negate: false,
            sweep_reload: 0,
            sweep_shift: 0,
            sweep_enabled: false,
            sweep_reload_flag: false,
        }
    }

    pub fn cycle_timer (&mut self) {
        if self.timer == 0 {
            self.timer = self.timer_reload;
            self.step = (self.step + 1) & 7;
        } else {
            self.timer -= 1;
        }
    }

    pub fn cycle_length (&mut self) {
        if self.length_enabled && self.length > 0 {
            self.length -= 1;
        }
    }

    pub fn cycle_envelope (&mut self) {
        if self.envelope_start {
            self.volume = 15;
            self.envelope_timer = self.envelope_timer_reload;
            self.envelope_start = false;
        } else if self.envelope_timer == 0 {
            if self.volume > 0 {
                self.volume -= 1;
            } else if self.envelope_loop {
                self.volume = 15;
            }

            self.envelope_timer = self.envelope_timer_reload;
        } else {
            self.envelope_timer -= 1;
        }
    }

    /// Compute the sweep target period as signed so we can detect underflow
    /// on negate without corrupting timer_reload.
    fn sweep_target (&self) -> i32 {
        let amount = (self.timer_reload >> self.sweep_shift) as i32;
        let current = self.timer_reload as i32;
        if self.sweep_negate {
            // Pulse 1 uses one's complement (extra -1); Pulse 2 uses two's complement.
            if self.id == 1 {
                current - amount - 1
            } else {
                current - amount
            }
        } else {
            current + amount
        }
    }

    /// The channel is muted whenever the current period is too low or the
    /// (hypothetical) sweep target period is too high.
    fn sweep_muting (&self) -> bool {
        self.timer_reload < 8 || self.sweep_target() > 0x7FF
    }

    pub fn cycle_sweep (&mut self) {
        // Only update the period when all conditions are met. Critically, a
        // zero shift count is a no-op — the previous implementation doubled
        // (or zeroed) the period, producing severe pitch wobble.
        if self.sweep_timer == 0
            && self.sweep_enabled
            && self.sweep_shift > 0
            && !self.sweep_muting()
        {
            self.timer_reload = self.sweep_target() as u16;
        }

        if self.sweep_timer == 0 || self.sweep_reload_flag {
            self.sweep_timer = self.sweep_reload;
            self.sweep_reload_flag = false;
        } else {
            self.sweep_timer -= 1;
        }
    }

    pub fn write_ctrl (&mut self, data: u8) {
        // Duty changes take effect immediately but do NOT reset the
        // sequencer position — this is what keeps mid-note volume/duty
        // changes click-free on real hardware.
        self.duty = (data & 0b1100_0000) >> 6;
        self.length_enabled = (data & 0b0010_0000) == 0;
        self.envelope_loop = (data & 0b0010_0000) > 0;
        self.envelope_enabled = (data & 0b0001_0000) == 0;
        self.envelope_timer_reload = data & 0b0000_1111;
        self.volume_constant = data & 0b0000_1111;
    }

    pub fn write_sweep (&mut self, data: u8) {
        self.sweep_enabled = (data & 0b1000_0000) > 0;
        self.sweep_reload = (data & 0b0111_0000) >> 4;
        self.sweep_negate = (data & 0b0000_1000) > 0;
        self.sweep_shift = data & 0b0000_0111;
        self.sweep_reload_flag = true;
    }

    pub fn write_lo (&mut self, data: u8) {
        self.timer_reload = (self.timer_reload & 0xFF00) | data as u16;
    }

    pub fn write_hi (&mut self, data: u8) {
        if self.length_enabled {
            self.length = super::LENGTH_TABLE[((data & 0b1111_1000) >> 3) as usize];
        }
        self.timer_reload = (self.timer_reload & 0x00FF) | (data as u16 & 0b0000_0111) << 8;
        self.timer = self.timer_reload;
        // $4003/$4007 restart the sequencer and envelope.
        self.step = 0;
        self.envelope_start = true;
    }

    pub fn output (&self) -> u8 {
        if self.length == 0 || self.sweep_muting() {
            0
        } else {
            let level = (PULSE_TABLE[self.duty as usize] >> self.step) & 1;
            level * if self.envelope_enabled { self.volume } else { self.volume_constant }
        }
    }

    pub fn enable (&mut self) {
        self.length_enabled = true;
    }

    pub fn disable (&mut self) {
        self.length_enabled = false;
        self.length = 0;
    }
}
