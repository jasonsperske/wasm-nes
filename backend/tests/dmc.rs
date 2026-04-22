/// Audio regression test: verify that the DMC-using bbbradsmith ROMs produce
/// non-silent output. Prior to DMC, `dac_tnd0`/`dac_tnd1` wrote to an
/// unimplemented channel and filtered to zero.
use wasm_nes::Emulator;

fn peak (samples: &[f32]) -> f32 {
    samples.iter().fold(0.0f32, |m, &s| m.max(s.abs()))
}

fn run_and_capture (rom: &[u8], warmup_frames: usize, capture_frames: usize) -> Vec<f32> {
    let mut nes = Emulator::new(rom.to_vec(), 48_000.0);
    for _ in 0..warmup_frames { nes.cycle_until_frame(); nes.get_audio(); }
    let mut samples = Vec::new();
    for _ in 0..capture_frames {
        nes.cycle_until_frame();
        samples.extend_from_slice(&nes.get_audio());
    }
    samples
}

#[test]
fn dac_tnd0_produces_audible_signal () {
    let rom = include_bytes!("../../tests/dac_tnd0.nes");
    let samples = run_and_capture(rom, 60, 300);
    let p = peak(&samples);
    println!("dac_tnd0 peak={p:.6}, samples={}", samples.len());
    assert!(p > 0.01, "dac_tnd0 near-silent (peak={:.6})", p);
}

#[test]
fn dac_tnd1_produces_audible_signal () {
    let rom = include_bytes!("../../tests/dac_tnd1.nes");
    let samples = run_and_capture(rom, 60, 300);
    let p = peak(&samples);
    println!("dac_tnd1 peak={p:.6}, samples={}", samples.len());
    assert!(p > 0.01, "dac_tnd1 near-silent (peak={:.6})", p);
}
