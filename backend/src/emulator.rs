use wasm_bindgen::prelude::*;
use crate::{bus, cpu, cpu::Interrupt, clock, input};

#[wasm_bindgen]
pub struct Emulator {
    pub (crate) cpu: cpu::Cpu,
    pub (crate) bus: bus::Bus,
    pub (crate) clock: clock::Clock,
    /// Integer counter for CPU/APU divider (CPU ticks every 3 PPU cycles)
    cpu_divider: u8,
    /// Integer accumulator for APU sample timing (fixed-point)
    sample_accumulator: u64,
    /// Sample period numerator: master clocks per sample = CLOCK_MASTER / sample_rate
    /// Scaled to PPU rate: PPU clocks per sample = CLOCK_PPU / sample_rate
    /// We use fixed-point: accumulate sample_rate per PPU tick, fire when >= CLOCK_PPU
    sample_rate: u64,
    sample_threshold: u64,
}

#[wasm_bindgen]
impl Emulator {
    pub fn new (rom: Vec<u8>, sample_rate: f64) -> Self {
        let mut emulator = Self {
            cpu: cpu::Cpu::new(),
            bus: bus::Bus::new(&rom, sample_rate),
            clock: clock::Clock::new(crate::clock::CLOCK_MASTER_NTSC),
            cpu_divider: 0,
            sample_accumulator: 0,
            sample_rate: sample_rate as u64,
            sample_threshold: crate::clock::CLOCK_PPU_NTSC as u64,
        };

        emulator.cpu.reset();

        emulator
    }

    /**
     * Run one PPU cycle (the fastest component).
     * CPU and APU tick every 3 PPU cycles (master/12 vs master/4).
     * Audio samples use an integer accumulator.
     */
    pub fn cycle (&mut self) {
        // PPU runs every cycle
        self.bus.ppu.cycle(&self.bus.cartridge, &mut self.cpu);
        self.bus.ppu.clock.cycles += 1;

        // CPU + APU run every 3 PPU cycles
        self.cpu_divider += 1;
        if self.cpu_divider == 3 {
            self.cpu_divider = 0;

            // CPU step (with DMA handling)
            self.cpu.clock.cycles += 1;
            let mut dma = self.bus.dma;
            match dma {
                Some (ref mut status) => {
                    if status.wait {
                        if self.cpu.clock.cycles % 2 == 1 {
                            status.wait = false;
                        }
                    } else {
                        if self.cpu.clock.cycles % 2 == 0 {
                            let address = ((status.page as u16) << 8) + status.count as u16;
                            status.read_buffer = self.bus.read(address);
                        } else {
                            self.bus.ppu.write_oam(status.read_buffer);
                            if status.count < u8::MAX {
                                status.count += 1;
                            } else {
                                dma = None;
                            }
                        }
                    }
                    self.bus.dma = dma;
                },
                None => {
                    self.cpu.cycle(&mut self.bus);
                },
            }

            // APU cycle (same rate as CPU)
            self.bus.apu.clock.cycles += 1;
            self.bus.apu.cycle(&mut self.cpu);

            // Service any pending DMC sample fetch. The DMC steals the CPU
            // bus for ~4 cycles per fetch on real hardware; we model that as
            // a stall added to the current instruction.
            if let Some(addr) = self.bus.apu.dmc.fetch_address() {
                let byte = self.bus.read(addr);
                self.bus.apu.dmc.complete_fetch(byte);
                self.cpu.cycles = self.cpu.cycles.saturating_add(4);
            }

            // Edge-trigger the CPU IRQ on DMC sample completion.
            if self.bus.apu.dmc.irq_pending {
                self.bus.apu.dmc.irq_pending = false;
                self.cpu.interrupt_request(Interrupt::IRQ);
            }
        }

        // APU sample accumulator (integer fixed-point)
        self.sample_accumulator += self.sample_rate;
        if self.sample_accumulator >= self.sample_threshold {
            self.sample_accumulator -= self.sample_threshold;
            self.bus.apu.sample();
        }
    }

    /**
     * Cycle until frame is rendered
     */
    pub fn cycle_until_frame (&mut self) {
        let frame = self.bus.ppu.frame;

        while frame == self.bus.ppu.frame {
            self.cycle();
        }
    }

    pub fn cycle_until_scanline (&mut self) {
        let cycle = self.bus.ppu.scanline;

        while cycle == self.bus.ppu.scanline {
            self.cycle();
        }
    }

    pub fn cycle_until_ppu (&mut self) {
        let cycle = self.bus.ppu.clock.cycles;

        while cycle == self.bus.ppu.clock.cycles {
            self.cycle();
        }
    }

    pub fn cycle_until_cpu (&mut self) {
        let cycle = self.cpu.clock.cycles;

        while cycle == self.cpu.clock.cycles {
            self.cycle();
        }
    }

    // pub fn set_rate (&mut self) {
    //     self.clock.rate = crate::util::CLOCK_MASTER_PAL;
    // }

    /// Mute/unmute individual APU channels for debugging.
    /// 0 = Pulse 1, 1 = Pulse 2, 2 = Triangle, 3 = Noise.
    pub fn set_channel_muted (&mut self, channel: u8, muted: bool) {
        self.bus.apu.set_channel_muted(channel as usize, muted);
    }

    pub fn update_controller (&mut self, player: usize, button: input::Button, pressed: bool) {
        let state = self.bus.controllers[player].peek().unwrap();
        let state = if pressed { state | button as u8 } else { state & !(button as u8)};

        self.bus.controllers[player].update(state);
    }

    /**
     * https://wiki.nesdev.org/w/index.php/Init_code
     */
    pub fn reset (&mut self) {
        self.cpu.reset();
        self.bus.apu.reset();
        self.clock.reset();
    }

    pub fn read (&mut self, address: u16) -> u8 {
        self.bus.read(address)
    }

    pub fn get_audio (&mut self) -> Vec<f32> {
        self.bus.apu.flush()
    }

    pub fn get_framebuffer (&self) -> js_sys::Uint8ClampedArray {
        unsafe { js_sys::Uint8ClampedArray::view(&self.bus.ppu.framebuffer) }
    }

    /// Save complete emulator state as a binary blob
    pub fn save_state (&self) -> Vec<u8> {
        let mut s: Vec<u8> = Vec::with_capacity(8192);

        // Magic + version. NESSAVE2 added DMC channel state at the end of the
        // APU section.
        s.extend_from_slice(b"NESSAVE2");

        // CPU state
        s.extend_from_slice(&self.cpu.pc.to_le_bytes());
        s.push(self.cpu.sp);
        s.push(self.cpu.a);
        s.push(self.cpu.x);
        s.push(self.cpu.y);
        s.push(self.cpu.status);
        s.extend_from_slice(&(self.cpu.cycles as u32).to_le_bytes());
        s.push(match self.cpu.interrupt {
            None => 0,
            Some(cpu::Interrupt::NMI) => 1,
            Some(cpu::Interrupt::IRQ) => 2,
            Some(cpu::Interrupt::RESET) => 3,
        });
        s.extend_from_slice(&(self.cpu.clock.cycles as u32).to_le_bytes());

        // Emulator dividers
        s.push(self.cpu_divider);
        s.extend_from_slice(&self.sample_accumulator.to_le_bytes());

        // RAM (2KB)
        let wram_len = self.bus.wram.len() as u32;
        s.extend_from_slice(&wram_len.to_le_bytes());
        s.extend_from_slice(&self.bus.wram);

        // PPU state
        s.push(self.bus.ppu.ctrl);
        s.push(self.bus.ppu.mask);
        s.push(self.bus.ppu.status);
        s.extend_from_slice(&self.bus.ppu.dot.to_le_bytes());
        s.extend_from_slice(&self.bus.ppu.scanline.to_le_bytes());
        s.extend_from_slice(&(self.bus.ppu.frame as u32).to_le_bytes());
        s.extend_from_slice(&(self.bus.ppu.clock.cycles as u32).to_le_bytes());
        // PPU internal registers
        s.extend_from_slice(&self.bus.ppu.cur_address.to_le_bytes());
        s.extend_from_slice(&self.bus.ppu.tmp_address.to_le_bytes());
        s.push(self.bus.ppu.scroll_x_fine);
        s.push(if self.bus.ppu.write_latch { 1 } else { 0 });
        s.push(self.bus.ppu.read_buffer);
        s.push(self.bus.ppu.oam_address);
        // PPU nametables
        let nt_len = self.bus.ppu.nametables.len() as u32;
        s.extend_from_slice(&nt_len.to_le_bytes());
        s.extend_from_slice(&self.bus.ppu.nametables);
        // PPU palettes
        let pal_len = self.bus.ppu.palettes.len() as u32;
        s.extend_from_slice(&pal_len.to_le_bytes());
        s.extend_from_slice(&self.bus.ppu.palettes);
        // PPU OAM
        let oam_len = self.bus.ppu.oam.len() as u32;
        s.extend_from_slice(&oam_len.to_le_bytes());
        s.extend_from_slice(&self.bus.ppu.oam);

        // APU state
        s.extend_from_slice(&(self.bus.apu.clock.cycles as u32).to_le_bytes());

        // DMC state (NESSAVE2+)
        let dmc = &self.bus.apu.dmc;
        s.extend_from_slice(&dmc.timer.to_le_bytes());
        s.push(dmc.output);
        s.extend_from_slice(&dmc.sample_address.to_le_bytes());
        s.extend_from_slice(&dmc.sample_length.to_le_bytes());
        s.extend_from_slice(&dmc.current_address.to_le_bytes());
        s.extend_from_slice(&dmc.bytes_remaining.to_le_bytes());
        s.push(dmc.sample_buffer.unwrap_or(0));
        s.push(if dmc.sample_buffer.is_some() { 1 } else { 0 });
        s.push(dmc.shift_register);
        s.push(dmc.bits_remaining);
        s.push(if dmc.silence { 1 } else { 0 });
        s.push(if dmc.irq_flag { 1 } else { 0 });
        s.push(if dmc.irq_pending { 1 } else { 0 });

        // Controller state
        s.push(self.bus.controllers[0].peek().unwrap_or(0));
        s.push(self.bus.controllers[1].peek().unwrap_or(0));

        // DMA state
        match self.bus.dma {
            Some(ref dma) => {
                s.push(1);
                s.push(dma.page);
                s.push(if dma.wait { 1 } else { 0 });
                s.push(dma.count);
                s.push(dma.read_buffer);
            },
            None => s.push(0),
        }

        // Cartridge PRG RAM
        let prgram_len = self.bus.cartridge.prg_ram.len() as u32;
        s.extend_from_slice(&prgram_len.to_le_bytes());
        s.extend_from_slice(&self.bus.cartridge.prg_ram);

        // CHR RAM (may be modified by mapper)
        let chr_len = self.bus.cartridge.chr.len() as u32;
        s.extend_from_slice(&chr_len.to_le_bytes());
        s.extend_from_slice(&self.bus.cartridge.chr);

        // Mapper state
        let mapper_state = self.bus.cartridge.mapper.save_state();
        let mapper_len = mapper_state.len() as u32;
        s.extend_from_slice(&mapper_len.to_le_bytes());
        s.extend_from_slice(&mapper_state);

        s
    }

    /// Load emulator state from a binary blob
    pub fn load_state (&mut self, data: &[u8]) {
        let read_u8 = |i: &mut usize, d: &[u8]| -> u8 { let v = d[*i]; *i += 1; v };
        let read_u16 = |i: &mut usize, d: &[u8]| -> u16 { let v = u16::from_le_bytes([d[*i], d[*i+1]]); *i += 2; v };
        let read_u32 = |i: &mut usize, d: &[u8]| -> u32 { let v = u32::from_le_bytes([d[*i], d[*i+1], d[*i+2], d[*i+3]]); *i += 4; v };
        let read_u64 = |i: &mut usize, d: &[u8]| -> u64 { let v = u64::from_le_bytes([d[*i], d[*i+1], d[*i+2], d[*i+3], d[*i+4], d[*i+5], d[*i+6], d[*i+7]]); *i += 8; v };

        // Magic check. NESSAVE1 is the legacy format (no DMC); NESSAVE2 adds
        // DMC channel state. Both are accepted on load.
        let has_dmc = match &data[0..8] {
            b"NESSAVE2" => true,
            b"NESSAVE1" => false,
            _ => return,
        };
        let mut i = 8;

        // CPU state
        self.cpu.pc = read_u16(&mut i, data);
        self.cpu.sp = read_u8(&mut i, data);
        self.cpu.a = read_u8(&mut i, data);
        self.cpu.x = read_u8(&mut i, data);
        self.cpu.y = read_u8(&mut i, data);
        self.cpu.status = read_u8(&mut i, data);
        self.cpu.cycles = read_u32(&mut i, data) as usize;
        self.cpu.interrupt = match read_u8(&mut i, data) {
            1 => Some(cpu::Interrupt::NMI),
            2 => Some(cpu::Interrupt::IRQ),
            3 => Some(cpu::Interrupt::RESET),
            _ => None,
        };
        self.cpu.clock.cycles = read_u32(&mut i, data) as usize;

        // Emulator dividers
        self.cpu_divider = read_u8(&mut i, data);
        self.sample_accumulator = read_u64(&mut i, data);

        // RAM
        let wram_len = read_u32(&mut i, data) as usize;
        self.bus.wram[..wram_len].copy_from_slice(&data[i..i+wram_len]);
        i += wram_len;

        // PPU state
        self.bus.ppu.ctrl = read_u8(&mut i, data);
        self.bus.ppu.mask = read_u8(&mut i, data);
        self.bus.ppu.status = read_u8(&mut i, data);
        self.bus.ppu.dot = read_u16(&mut i, data);
        self.bus.ppu.scanline = read_u16(&mut i, data);
        self.bus.ppu.frame = read_u32(&mut i, data) as usize;
        self.bus.ppu.clock.cycles = read_u32(&mut i, data) as usize;
        self.bus.ppu.cur_address = read_u16(&mut i, data);
        self.bus.ppu.tmp_address = read_u16(&mut i, data);
        self.bus.ppu.scroll_x_fine = read_u8(&mut i, data);
        self.bus.ppu.write_latch = read_u8(&mut i, data) != 0;
        self.bus.ppu.read_buffer = read_u8(&mut i, data);
        self.bus.ppu.oam_address = read_u8(&mut i, data);
        // PPU nametables
        let nt_len = read_u32(&mut i, data) as usize;
        self.bus.ppu.nametables[..nt_len].copy_from_slice(&data[i..i+nt_len]);
        i += nt_len;
        // PPU palettes
        let pal_len = read_u32(&mut i, data) as usize;
        self.bus.ppu.palettes[..pal_len].copy_from_slice(&data[i..i+pal_len]);
        i += pal_len;
        // PPU OAM
        let oam_len = read_u32(&mut i, data) as usize;
        self.bus.ppu.oam[..oam_len].copy_from_slice(&data[i..i+oam_len]);
        i += oam_len;

        // APU state
        self.bus.apu.clock.cycles = read_u32(&mut i, data) as usize;

        // DMC state (NESSAVE2+). For legacy NESSAVE1 saves, leave the DMC at
        // its constructor defaults (silent, idle).
        if has_dmc {
            let dmc = &mut self.bus.apu.dmc;
            dmc.timer = read_u16(&mut i, data);
            dmc.output = read_u8(&mut i, data);
            dmc.sample_address = read_u16(&mut i, data);
            dmc.sample_length = read_u16(&mut i, data);
            dmc.current_address = read_u16(&mut i, data);
            dmc.bytes_remaining = read_u16(&mut i, data);
            let buffer_byte = read_u8(&mut i, data);
            let has_buffer = read_u8(&mut i, data) != 0;
            dmc.sample_buffer = if has_buffer { Some(buffer_byte) } else { None };
            dmc.shift_register = read_u8(&mut i, data);
            dmc.bits_remaining = read_u8(&mut i, data);
            dmc.silence = read_u8(&mut i, data) != 0;
            dmc.irq_flag = read_u8(&mut i, data) != 0;
            dmc.irq_pending = read_u8(&mut i, data) != 0;
        }

        // Controller state
        self.bus.controllers[0].update(read_u8(&mut i, data));
        self.bus.controllers[1].update(read_u8(&mut i, data));

        // DMA state
        if read_u8(&mut i, data) == 1 {
            self.bus.dma = Some(bus::Dma {
                page: read_u8(&mut i, data),
                wait: read_u8(&mut i, data) != 0,
                count: read_u8(&mut i, data),
                read_buffer: read_u8(&mut i, data),
            });
        } else {
            self.bus.dma = None;
        }

        // Cartridge PRG RAM
        let prgram_len = read_u32(&mut i, data) as usize;
        self.bus.cartridge.prg_ram[..prgram_len].copy_from_slice(&data[i..i+prgram_len]);
        i += prgram_len;

        // CHR
        let chr_len = read_u32(&mut i, data) as usize;
        self.bus.cartridge.chr[..chr_len].copy_from_slice(&data[i..i+chr_len]);
        i += chr_len;

        // Mapper state
        let mapper_len = read_u32(&mut i, data) as usize;
        self.bus.cartridge.mapper.load_state(&data[i..i+mapper_len]);
    }
}
