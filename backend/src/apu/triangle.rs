const TRIANGLE_SEQUENCE: [u8; 32] = [
    15, 14, 13, 12, 11, 10,  9,  8,  7,  6,  5,  4,  3,  2,  1,  0,
     0,  1,  2,  3,  4,  5,  6,  7,  8,  9, 10, 11, 12, 13, 14, 15,
];

/**
 * https://wiki.nesdev.org/w/index.php/APU_Triangle
 *
 * Clocked at the CPU rate (not CPU/2 like pulses). Gated by a length counter
 * AND a linear counter — both must be non-zero for the sequencer to advance.
 * Output is a 4-bit value (0-15); there is no volume control.
 */
#[derive(Clone)]
pub struct Triangle {
    timer: u16,
    timer_reload: u16,
    step: u8,
    pub length: u8,
    length_enabled: bool,
    linear_counter: u8,
    linear_reload: u8,
    linear_reload_flag: bool,
    halt: bool,
}

impl Triangle {
    pub fn new () -> Self {
        Self {
            timer: 0,
            timer_reload: 0,
            step: 0,
            length: 0,
            length_enabled: true,
            linear_counter: 0,
            linear_reload: 0,
            linear_reload_flag: false,
            halt: false,
        }
    }

    pub fn cycle_timer (&mut self) {
        if self.timer == 0 {
            self.timer = self.timer_reload;
            // Only advance the sequence if both counters are armed. This
            // prevents a DC offset when the channel is off.
            if self.length > 0 && self.linear_counter > 0 {
                self.step = (self.step + 1) & 0x1F;
            }
        } else {
            self.timer -= 1;
        }
    }

    pub fn cycle_length (&mut self) {
        if !self.halt && self.length > 0 {
            self.length -= 1;
        }
    }

    /// Called on the quarter-frame clock.
    pub fn cycle_linear (&mut self) {
        if self.linear_reload_flag {
            self.linear_counter = self.linear_reload;
        } else if self.linear_counter > 0 {
            self.linear_counter -= 1;
        }
        if !self.halt {
            self.linear_reload_flag = false;
        }
    }

    pub fn write_ctrl (&mut self, data: u8) {
        self.halt = (data & 0x80) != 0;
        self.length_enabled = (data & 0x80) == 0;
        self.linear_reload = data & 0x7F;
    }

    pub fn write_lo (&mut self, data: u8) {
        self.timer_reload = (self.timer_reload & 0xFF00) | data as u16;
    }

    pub fn write_hi (&mut self, data: u8) {
        self.timer_reload = (self.timer_reload & 0x00FF) | ((data as u16 & 0x07) << 8);
        if self.length_enabled {
            self.length = super::LENGTH_TABLE[((data >> 3) & 0x1F) as usize];
        }
        self.linear_reload_flag = true;
    }

    pub fn output (&self) -> u8 {
        // Mute ultrasonic frequencies to avoid DC clicks and aliased hash.
        // The sequence itself produces audible clicks when timer_reload is
        // very small because the output holds one sample per half-cycle.
        if self.timer_reload < 2 {
            0
        } else {
            TRIANGLE_SEQUENCE[self.step as usize]
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
