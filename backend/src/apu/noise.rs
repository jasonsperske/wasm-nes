/// NTSC noise channel timer periods, indexed by the low 4 bits of $400E.
const NOISE_PERIODS_NTSC: [u16; 16] = [
    4, 8, 16, 32, 64, 96, 128, 160, 202, 254, 380, 508, 762, 1016, 2034, 4068,
];

/**
 * https://wiki.nesdev.org/w/index.php/APU_Noise
 *
 * A 15-bit linear feedback shift register produces pseudo-random bits clocked
 * by a period timer. Envelope and length counter work like the pulse channels.
 */
#[derive(Clone)]
pub struct Noise {
    lfsr: u16,
    mode: bool,
    timer: u16,
    timer_reload: u16,
    pub length: u8,
    length_enabled: bool,
    volume: u8,
    volume_constant: u8,
    envelope_start: bool,
    envelope_timer: u8,
    envelope_timer_reload: u8,
    envelope_loop: bool,
    envelope_enabled: bool,
}

impl Noise {
    pub fn new () -> Self {
        Self {
            // LFSR must be initialized non-zero — all zeros is a dead state.
            lfsr: 1,
            mode: false,
            timer: 0,
            timer_reload: 0,
            length: 0,
            length_enabled: true,
            volume: 0,
            volume_constant: 15,
            envelope_start: false,
            envelope_timer: 0,
            envelope_timer_reload: 0,
            envelope_loop: false,
            envelope_enabled: true,
        }
    }

    pub fn cycle_timer (&mut self) {
        if self.timer == 0 {
            self.timer = self.timer_reload;
            let bit0 = self.lfsr & 1;
            let other = if self.mode {
                (self.lfsr >> 6) & 1
            } else {
                (self.lfsr >> 1) & 1
            };
            let feedback = bit0 ^ other;
            self.lfsr = (self.lfsr >> 1) | (feedback << 14);
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

    pub fn write_ctrl (&mut self, data: u8) {
        self.length_enabled = (data & 0b0010_0000) == 0;
        self.envelope_loop = (data & 0b0010_0000) != 0;
        self.envelope_enabled = (data & 0b0001_0000) == 0;
        self.envelope_timer_reload = data & 0b0000_1111;
        self.volume_constant = data & 0b0000_1111;
    }

    pub fn write_mode (&mut self, data: u8) {
        self.mode = (data & 0b1000_0000) != 0;
        self.timer_reload = NOISE_PERIODS_NTSC[(data & 0b0000_1111) as usize];
    }

    pub fn write_length (&mut self, data: u8) {
        if self.length_enabled {
            self.length = super::LENGTH_TABLE[((data >> 3) & 0x1F) as usize];
        }
        self.envelope_start = true;
    }

    pub fn output (&self) -> u8 {
        if self.length == 0 || (self.lfsr & 1) != 0 {
            0
        } else if self.envelope_enabled {
            self.volume
        } else {
            self.volume_constant
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
