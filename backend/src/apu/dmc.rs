/// NTSC DMC rate periods (CPU cycles per output bit), indexed by the low 4
/// bits of `$4010`. Source: nesdev wiki (APU DMC).
const RATE_TABLE_NTSC: [u16; 16] = [
    428, 380, 340, 320, 286, 254, 226, 214, 190, 160, 142, 128, 106, 84, 72, 54,
];

/**
 * https://wiki.nesdev.org/w/index.php/APU_DMC
 *
 * The DMC plays 7-bit PCM samples streamed from CPU memory. It has three
 * concurrently-running pieces:
 *
 * 1. The output unit shifts a byte one bit at a time at a programmable rate;
 *    each bit nudges the 7-bit DAC up or down by 2 (saturating at 0/127).
 * 2. The memory reader fetches the next sample byte into a one-byte buffer
 *    whenever the buffer is empty and bytes-remaining is non-zero. Fetches
 *    happen on the CPU bus and stall the CPU for a few cycles.
 * 3. The DAC level can also be set directly via `$4011`, which is what the
 *    bbbradsmith DAC linearity tests exercise.
 *
 * The actual bus read is performed by the emulator (see `fetch_address` /
 * `complete_fetch`) since the APU does not own the bus.
 */
#[derive(Clone)]
pub struct Dmc {
    irq_enable: bool,
    pub irq_flag: bool,
    loop_flag: bool,
    pub timer: u16,
    timer_reload: u16,
    /// 7-bit DAC level (0..=127). Directly writable via `$4011`.
    pub output: u8,
    pub sample_address: u16,
    pub sample_length: u16,
    pub current_address: u16,
    pub bytes_remaining: u16,
    pub sample_buffer: Option<u8>,
    pub shift_register: u8,
    pub bits_remaining: u8,
    pub silence: bool,
    /// Latched after the last byte of a sample is consumed; used by the
    /// emulator to raise an edge-triggered IRQ to the CPU.
    pub irq_pending: bool,
}

impl Dmc {
    pub fn new () -> Self {
        Self {
            irq_enable: false,
            irq_flag: false,
            loop_flag: false,
            timer: 0,
            timer_reload: RATE_TABLE_NTSC[0] - 1,
            output: 0,
            sample_address: 0xC000,
            sample_length: 1,
            current_address: 0xC000,
            bytes_remaining: 0,
            sample_buffer: None,
            shift_register: 0,
            bits_remaining: 8,
            silence: true,
            irq_pending: false,
        }
    }

    pub fn write_ctrl (&mut self, data: u8) {
        self.irq_enable = (data & 0x80) != 0;
        self.loop_flag = (data & 0x40) != 0;
        self.timer_reload = RATE_TABLE_NTSC[(data & 0x0F) as usize] - 1;
        // Disabling IRQ also clears any pending IRQ flag.
        if !self.irq_enable {
            self.irq_flag = false;
        }
    }

    pub fn write_dac (&mut self, data: u8) {
        self.output = data & 0x7F;
    }

    pub fn write_address (&mut self, data: u8) {
        // Sample address = $C000 + (data * 64)
        self.sample_address = 0xC000 | ((data as u16) << 6);
    }

    pub fn write_length (&mut self, data: u8) {
        // Sample length = (data * 16) + 1 bytes
        self.sample_length = ((data as u16) << 4) | 1;
    }

    /// Called when bit 4 of `$4015` transitions to 1. Restarts playback only
    /// if the channel is currently idle; otherwise the in-flight sample
    /// continues unmodified (per nesdev).
    pub fn enable (&mut self) {
        if self.bytes_remaining == 0 {
            self.current_address = self.sample_address;
            self.bytes_remaining = self.sample_length;
        }
    }

    /// Called when bit 4 of `$4015` transitions to 0. Silences playback at
    /// the next byte boundary by zeroing bytes-remaining.
    pub fn disable (&mut self) {
        self.bytes_remaining = 0;
    }

    pub fn active (&self) -> bool {
        self.bytes_remaining > 0
    }

    pub fn clear_irq (&mut self) {
        self.irq_flag = false;
    }

    pub fn cycle_timer (&mut self) {
        if self.timer == 0 {
            self.timer = self.timer_reload;
            self.clock_output();
        } else {
            self.timer -= 1;
        }
    }

    fn clock_output (&mut self) {
        if !self.silence {
            if (self.shift_register & 1) != 0 {
                if self.output <= 125 {
                    self.output += 2;
                }
            } else if self.output >= 2 {
                self.output -= 2;
            }
        }

        self.shift_register >>= 1;
        self.bits_remaining -= 1;

        if self.bits_remaining == 0 {
            self.start_output_cycle();
        }
    }

    fn start_output_cycle (&mut self) {
        self.bits_remaining = 8;
        match self.sample_buffer.take() {
            Some(byte) => {
                self.shift_register = byte;
                self.silence = false;
            },
            None => {
                self.silence = true;
            },
        }
    }

    /// Address the memory reader wants to fetch from, or `None` if the
    /// sample buffer is already full (or there's nothing to play).
    pub fn fetch_address (&self) -> Option<u16> {
        if self.sample_buffer.is_none() && self.bytes_remaining > 0 {
            Some(self.current_address)
        } else {
            None
        }
    }

    /// Called by the emulator after performing a bus read for `fetch_address`.
    /// Loads the byte into the sample buffer, advances the read pointer, and
    /// handles loop/IRQ when the sample ends.
    pub fn complete_fetch (&mut self, byte: u8) {
        self.sample_buffer = Some(byte);
        self.current_address = if self.current_address == 0xFFFF {
            0x8000
        } else {
            self.current_address + 1
        };
        self.bytes_remaining -= 1;
        if self.bytes_remaining == 0 {
            if self.loop_flag {
                self.current_address = self.sample_address;
                self.bytes_remaining = self.sample_length;
            } else if self.irq_enable {
                self.irq_flag = true;
                self.irq_pending = true;
            }
        }
    }

    pub fn output (&self) -> u8 {
        self.output
    }
}
