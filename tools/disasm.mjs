#!/usr/bin/env node
/**
 * 6502 iNES ROM Disassembler
 *
 * Usage:
 *   node tools/disasm.mjs rom.nes                  # full disassembly
 *   node tools/disasm.mjs rom.nes --addr 0x8000    # start at address
 *   node tools/disasm.mjs rom.nes --count 100      # limit instructions
 *   node tools/disasm.mjs rom.nes --vectors        # show reset/NMI/IRQ vectors
 *   node tools/disasm.mjs rom.nes --ram-refs        # show all RAM address references
 *   node tools/disasm.mjs rom.nes --search STA     # find specific opcodes
 *   node tools/disasm.mjs rom.nes --header          # show iNES header info only
 */

import { readFileSync } from 'fs';

// ── 6502 Opcode Table ──────────────────────────────────────────────────────

const MODES = {
    IMP: 0, ACC: 1, IMM: 2, REL: 3,
    ABS: 4, ABX: 5, ABY: 6,
    ZPG: 7, ZPX: 8, ZPY: 9,
    IND: 10, IDX: 11, IDY: 12,
};

// Size in bytes for each addressing mode (including opcode)
const MODE_SIZE = {
    [MODES.IMP]: 1, [MODES.ACC]: 1, [MODES.IMM]: 2, [MODES.REL]: 2,
    [MODES.ABS]: 3, [MODES.ABX]: 3, [MODES.ABY]: 3,
    [MODES.ZPG]: 2, [MODES.ZPX]: 2, [MODES.ZPY]: 2,
    [MODES.IND]: 3, [MODES.IDX]: 2, [MODES.IDY]: 2,
};

// [mnemonic, addressing mode, illegal?]
const OPCODES = new Array(256).fill(null);
function op(code, name, mode, illegal = false) {
    OPCODES[code] = { name, mode, size: MODE_SIZE[mode], illegal };
}

// ADC
op(0x69,'ADC',MODES.IMM); op(0x65,'ADC',MODES.ZPG); op(0x75,'ADC',MODES.ZPX);
op(0x6D,'ADC',MODES.ABS); op(0x7D,'ADC',MODES.ABX); op(0x79,'ADC',MODES.ABY);
op(0x61,'ADC',MODES.IDX); op(0x71,'ADC',MODES.IDY);
// AND
op(0x29,'AND',MODES.IMM); op(0x25,'AND',MODES.ZPG); op(0x35,'AND',MODES.ZPX);
op(0x2D,'AND',MODES.ABS); op(0x3D,'AND',MODES.ABX); op(0x39,'AND',MODES.ABY);
op(0x21,'AND',MODES.IDX); op(0x31,'AND',MODES.IDY);
// ASL
op(0x0A,'ASL',MODES.ACC); op(0x06,'ASL',MODES.ZPG); op(0x16,'ASL',MODES.ZPX);
op(0x0E,'ASL',MODES.ABS); op(0x1E,'ASL',MODES.ABX);
// Branch
op(0x90,'BCC',MODES.REL); op(0xB0,'BCS',MODES.REL); op(0xF0,'BEQ',MODES.REL);
op(0x30,'BMI',MODES.REL); op(0xD0,'BNE',MODES.REL); op(0x10,'BPL',MODES.REL);
op(0x50,'BVC',MODES.REL); op(0x70,'BVS',MODES.REL);
// BIT
op(0x24,'BIT',MODES.ZPG); op(0x2C,'BIT',MODES.ABS);
// BRK
op(0x00,'BRK',MODES.IMP);
// Clear/Set flags
op(0x18,'CLC',MODES.IMP); op(0xD8,'CLD',MODES.IMP); op(0x58,'CLI',MODES.IMP); op(0xB8,'CLV',MODES.IMP);
op(0x38,'SEC',MODES.IMP); op(0xF8,'SED',MODES.IMP); op(0x78,'SEI',MODES.IMP);
// CMP
op(0xC9,'CMP',MODES.IMM); op(0xC5,'CMP',MODES.ZPG); op(0xD5,'CMP',MODES.ZPX);
op(0xCD,'CMP',MODES.ABS); op(0xDD,'CMP',MODES.ABX); op(0xD9,'CMP',MODES.ABY);
op(0xC1,'CMP',MODES.IDX); op(0xD1,'CMP',MODES.IDY);
// CPX
op(0xE0,'CPX',MODES.IMM); op(0xE4,'CPX',MODES.ZPG); op(0xEC,'CPX',MODES.ABS);
// CPY
op(0xC0,'CPY',MODES.IMM); op(0xC4,'CPY',MODES.ZPG); op(0xCC,'CPY',MODES.ABS);
// DEC
op(0xC6,'DEC',MODES.ZPG); op(0xD6,'DEC',MODES.ZPX); op(0xCE,'DEC',MODES.ABS); op(0xDE,'DEC',MODES.ABX);
op(0xCA,'DEX',MODES.IMP); op(0x88,'DEY',MODES.IMP);
// EOR
op(0x49,'EOR',MODES.IMM); op(0x45,'EOR',MODES.ZPG); op(0x55,'EOR',MODES.ZPX);
op(0x4D,'EOR',MODES.ABS); op(0x5D,'EOR',MODES.ABX); op(0x59,'EOR',MODES.ABY);
op(0x41,'EOR',MODES.IDX); op(0x51,'EOR',MODES.IDY);
// INC
op(0xE6,'INC',MODES.ZPG); op(0xF6,'INC',MODES.ZPX); op(0xEE,'INC',MODES.ABS); op(0xFE,'INC',MODES.ABX);
op(0xE8,'INX',MODES.IMP); op(0xC8,'INY',MODES.IMP);
// JMP
op(0x4C,'JMP',MODES.ABS); op(0x6C,'JMP',MODES.IND);
// JSR
op(0x20,'JSR',MODES.ABS);
// LDA
op(0xA9,'LDA',MODES.IMM); op(0xA5,'LDA',MODES.ZPG); op(0xB5,'LDA',MODES.ZPX);
op(0xAD,'LDA',MODES.ABS); op(0xBD,'LDA',MODES.ABX); op(0xB9,'LDA',MODES.ABY);
op(0xA1,'LDA',MODES.IDX); op(0xB1,'LDA',MODES.IDY);
// LDX
op(0xA2,'LDX',MODES.IMM); op(0xA6,'LDX',MODES.ZPG); op(0xB6,'LDX',MODES.ZPY);
op(0xAE,'LDX',MODES.ABS); op(0xBE,'LDX',MODES.ABY);
// LDY
op(0xA0,'LDY',MODES.IMM); op(0xA4,'LDY',MODES.ZPG); op(0xB4,'LDY',MODES.ZPX);
op(0xAC,'LDY',MODES.ABS); op(0xBC,'LDY',MODES.ABX);
// LSR
op(0x4A,'LSR',MODES.ACC); op(0x46,'LSR',MODES.ZPG); op(0x56,'LSR',MODES.ZPX);
op(0x4E,'LSR',MODES.ABS); op(0x5E,'LSR',MODES.ABX);
// NOP
op(0xEA,'NOP',MODES.IMP);
// ORA
op(0x09,'ORA',MODES.IMM); op(0x05,'ORA',MODES.ZPG); op(0x15,'ORA',MODES.ZPX);
op(0x0D,'ORA',MODES.ABS); op(0x1D,'ORA',MODES.ABX); op(0x19,'ORA',MODES.ABY);
op(0x01,'ORA',MODES.IDX); op(0x11,'ORA',MODES.IDY);
// Stack
op(0x48,'PHA',MODES.IMP); op(0x08,'PHP',MODES.IMP);
op(0x68,'PLA',MODES.IMP); op(0x28,'PLP',MODES.IMP);
// ROL/ROR
op(0x2A,'ROL',MODES.ACC); op(0x26,'ROL',MODES.ZPG); op(0x36,'ROL',MODES.ZPX);
op(0x2E,'ROL',MODES.ABS); op(0x3E,'ROL',MODES.ABX);
op(0x6A,'ROR',MODES.ACC); op(0x66,'ROR',MODES.ZPG); op(0x76,'ROR',MODES.ZPX);
op(0x6E,'ROR',MODES.ABS); op(0x7E,'ROR',MODES.ABX);
// Return
op(0x40,'RTI',MODES.IMP); op(0x60,'RTS',MODES.IMP);
// SBC
op(0xE9,'SBC',MODES.IMM); op(0xE5,'SBC',MODES.ZPG); op(0xF5,'SBC',MODES.ZPX);
op(0xED,'SBC',MODES.ABS); op(0xFD,'SBC',MODES.ABX); op(0xF9,'SBC',MODES.ABY);
op(0xE1,'SBC',MODES.IDX); op(0xF1,'SBC',MODES.IDY);
// STA
op(0x85,'STA',MODES.ZPG); op(0x95,'STA',MODES.ZPX);
op(0x8D,'STA',MODES.ABS); op(0x9D,'STA',MODES.ABX); op(0x99,'STA',MODES.ABY);
op(0x81,'STA',MODES.IDX); op(0x91,'STA',MODES.IDY);
// STX
op(0x86,'STX',MODES.ZPG); op(0x96,'STX',MODES.ZPY); op(0x8E,'STX',MODES.ABS);
// STY
op(0x84,'STY',MODES.ZPG); op(0x94,'STY',MODES.ZPX); op(0x8C,'STY',MODES.ABS);
// Transfer
op(0xAA,'TAX',MODES.IMP); op(0xA8,'TAY',MODES.IMP);
op(0xBA,'TSX',MODES.IMP); op(0x8A,'TXA',MODES.IMP);
op(0x9A,'TXS',MODES.IMP); op(0x98,'TYA',MODES.IMP);

// Fill remaining as illegal NOPs with appropriate sizes
for (let i = 0; i < 256; i++) {
    if (!OPCODES[i]) {
        // Guess size from bit patterns for undocumented opcodes
        let size = 1;
        const lo = i & 0x0F;
        const hi = (i >> 4) & 0x0F;
        if (lo === 0x0B || lo === 0x02) size = 2;
        else if (lo === 0x03 || lo === 0x07 || lo === 0x0F) {
            size = (hi & 1) ? 3 : 2; // rough heuristic
        }
        else if (lo === 0x04 || lo === 0x0C) {
            size = (i & 0x10) ? 2 : ((i >= 0x80) ? 2 : 2);
            if (lo === 0x0C && !(i & 0x10)) size = 3;
        }
        OPCODES[i] = { name: '???', mode: MODES.IMP, size, illegal: true };
    }
}

// ── iNES Parser ────────────────────────────────────────────────────────────

function parseINES(buf) {
    if (buf[0] !== 0x4E || buf[1] !== 0x45 || buf[2] !== 0x53 || buf[3] !== 0x1A) {
        throw new Error('Not a valid iNES ROM (missing NES\\x1A header)');
    }
    const prgBanks = buf[4];
    const chrBanks = buf[5];
    const flags6 = buf[6];
    const flags7 = buf[7];
    const mapper = (flags7 & 0xF0) | (flags6 >> 4);
    const mirroring = (flags6 & 1) ? 'Vertical' : 'Horizontal';
    const battery = !!(flags6 & 2);
    const trainer = !!(flags6 & 4);
    const headerSize = 16 + (trainer ? 512 : 0);
    const prgSize = prgBanks * 16384;
    const chrSize = chrBanks * 8192;
    const prg = buf.subarray(headerSize, headerSize + prgSize);
    const chr = buf.subarray(headerSize + prgSize, headerSize + prgSize + chrSize);

    return {
        prgBanks, chrBanks, mapper, mirroring, battery, trainer,
        prgSize, chrSize, prg, chr,
    };
}

// ── Disassembler ───────────────────────────────────────────────────────────

function hex8(v) { return '$' + v.toString(16).toUpperCase().padStart(2, '0'); }
function hex16(v) { return '$' + v.toString(16).toUpperCase().padStart(4, '0'); }

function formatOperand(opInfo, bytes, pc) {
    const mode = opInfo.mode;
    if (bytes.length === 1) {
        switch (mode) {
            case MODES.ACC: return 'A';
            default: return '';
        }
    }
    if (bytes.length === 2) {
        const val = bytes[1];
        switch (mode) {
            case MODES.IMM: return `#${hex8(val)}`;
            case MODES.ZPG: return hex8(val);
            case MODES.ZPX: return `${hex8(val)},X`;
            case MODES.ZPY: return `${hex8(val)},Y`;
            case MODES.IDX: return `(${hex8(val)},X)`;
            case MODES.IDY: return `(${hex8(val)}),Y`;
            case MODES.REL: {
                const offset = val < 128 ? val : val - 256;
                const target = (pc + 2 + offset) & 0xFFFF;
                return hex16(target);
            }
            default: return hex8(val);
        }
    }
    if (bytes.length === 3) {
        const addr = bytes[1] | (bytes[2] << 8);
        switch (mode) {
            case MODES.ABS: return hex16(addr);
            case MODES.ABX: return `${hex16(addr)},X`;
            case MODES.ABY: return `${hex16(addr)},Y`;
            case MODES.IND: return `(${hex16(addr)})`;
            default: return hex16(addr);
        }
    }
    return '';
}

function getReferencedAddress(opInfo, bytes, pc) {
    const mode = opInfo.mode;
    if (bytes.length === 2) {
        switch (mode) {
            case MODES.ZPG: case MODES.ZPX: case MODES.ZPY:
            case MODES.IDX: case MODES.IDY:
                return bytes[1]; // zero page address
            case MODES.REL: {
                const offset = bytes[1] < 128 ? bytes[1] : bytes[1] - 256;
                return (pc + 2 + offset) & 0xFFFF;
            }
        }
    }
    if (bytes.length === 3) {
        return bytes[1] | (bytes[2] << 8);
    }
    return null;
}

function disassemble(prg, baseAddr, opts = {}) {
    const { count, search, ramRefs } = opts;
    const results = [];
    const ramAccesses = new Map(); // addr -> [{pc, mnemonic, operand}]
    let offset = 0;
    let n = 0;

    while (offset < prg.length) {
        if (count && n >= count) break;

        const pc = baseAddr + offset;
        const opcode = prg[offset];
        const opInfo = OPCODES[opcode];
        const size = opInfo.size;

        if (offset + size > prg.length) break;

        const bytes = prg.subarray(offset, offset + size);
        const operand = formatOperand(opInfo, bytes, pc);
        const bytesHex = Array.from(bytes).map(b => b.toString(16).toUpperCase().padStart(2, '0')).join(' ');
        const illegal = opInfo.illegal ? '*' : ' ';
        const line = `${hex16(pc)}  ${bytesHex.padEnd(8)} ${illegal}${opInfo.name} ${operand}`;

        // Track RAM references
        const refAddr = getReferencedAddress(opInfo, bytes, pc);
        if (refAddr !== null && refAddr < 0x0800) {
            if (!ramAccesses.has(refAddr)) ramAccesses.set(refAddr, []);
            ramAccesses.get(refAddr).push({ pc, mnemonic: opInfo.name, operand });
        }

        if (search) {
            if (opInfo.name === search.toUpperCase() || operand.includes(search.toUpperCase())) {
                results.push(line);
            }
        } else if (!ramRefs) {
            results.push(line);
        }

        offset += size;
        n++;
    }

    return { lines: results, ramAccesses };
}

// ── Known NES RAM Annotations ──────────────────────────────────────────────

const SMB_RAM_LABELS = {
    0x000E: 'Player_State (0=loading,1=alive,2=dead,3=entering_pipe,...)',
    0x000F: 'Player_MovingDir',
    0x001D: 'Player_FacingDir',
    0x0033: 'Player_Y_HighPos',
    0x00B5: 'Player_Y_Scroll',
    0x0086: 'Player_X_Position',
    0x03AD: 'Player_X_Position_Hi',
    0x00CE: 'Player_Y_Position',
    0x0700: 'GameMode (0=title,1=playing,2=victory)',
    0x0704: 'Secondary_GameMode',
    0x0705: 'OperMode_Task',
    0x0706: 'GameEngineSubroutine',
    0x0747: 'AreaType (0=water,1=ground,2=underground,3=castle)',
    0x0750: 'CurrentWorld (0-7)',
    0x0751: 'CurrentLevel (0-3)',
    0x0752: 'AreaPointer',
    0x0753: 'Rept_AreaPointer',
    0x0754: 'CurrentColumnPos',
    0x0756: 'LevelNumber',
    0x0757: 'WorldNumber',
    0x075F: 'ScreenRoutineTask',
    0x0760: 'AltEntranceControl',
    0x0761: 'AreaStyle (pipe/vine/etc transition)',
    0x0770: 'ScreenLeft_PageLoc',
    0x071A: 'Player_Pipe_Y_Position',
    0x074E: 'WarpZoneControl',
};

// ── Main ───────────────────────────────────────────────────────────────────

const args = process.argv.slice(2);
if (args.length === 0) {
    console.log('Usage: node tools/disasm.mjs <rom.nes> [options]');
    console.log('');
    console.log('Options:');
    console.log('  --header         Show iNES header info only');
    console.log('  --vectors        Show interrupt vectors (RESET/NMI/IRQ)');
    console.log('  --addr 0xNNNN   Start disassembly at address (default: reset vector)');
    console.log('  --count N       Limit to N instructions');
    console.log('  --search OPC    Search for opcode (e.g., STA, JMP, JSR)');
    console.log('  --ram-refs       Show all RAM ($0000-$07FF) references with annotations');
    console.log('  --smb            Annotate known Super Mario Bros RAM addresses');
    process.exit(0);
}

const romPath = args[0];
const buf = readFileSync(romPath);
const rom = parseINES(buf);

// Parse flags
const showHeader = args.includes('--header');
const showVectors = args.includes('--vectors');
const showRamRefs = args.includes('--ram-refs');
const showSMB = args.includes('--smb');
const addrIdx = args.indexOf('--addr');
const countIdx = args.indexOf('--count');
const searchIdx = args.indexOf('--search');

const startAddr = addrIdx >= 0 ? parseInt(args[addrIdx + 1]) : null;
const count = countIdx >= 0 ? parseInt(args[countIdx + 1]) : null;
const search = searchIdx >= 0 ? args[searchIdx + 1] : null;

// Determine PRG base address (depends on bank count)
const prgBaseAddr = rom.prgBanks === 1 ? 0xC000 : 0x8000;

// Header
console.log('=== iNES ROM Header ===');
console.log(`PRG ROM: ${rom.prgBanks} x 16KB = ${rom.prgSize} bytes`);
console.log(`CHR ROM: ${rom.chrBanks} x 8KB = ${rom.chrSize} bytes`);
console.log(`Mapper:  ${rom.mapper}`);
console.log(`Mirror:  ${rom.mirroring}`);
console.log(`Battery: ${rom.battery}`);
console.log(`Trainer: ${rom.trainer}`);
console.log(`PRG Address Range: ${hex16(prgBaseAddr)} - ${hex16(prgBaseAddr + rom.prgSize - 1)}`);
console.log('');

if (showHeader) process.exit(0);

// Vectors (last 6 bytes of PRG)
const nmiAddr = rom.prg[rom.prgSize - 6] | (rom.prg[rom.prgSize - 5] << 8);
const resetAddr = rom.prg[rom.prgSize - 4] | (rom.prg[rom.prgSize - 3] << 8);
const irqAddr = rom.prg[rom.prgSize - 2] | (rom.prg[rom.prgSize - 1] << 8);

if (showVectors) {
    console.log('=== Interrupt Vectors ===');
    console.log(`NMI:   ${hex16(nmiAddr)}`);
    console.log(`RESET: ${hex16(resetAddr)}`);
    console.log(`IRQ:   ${hex16(irqAddr)}`);
    console.log('');
}

// Disassemble
const effectiveStart = startAddr ?? resetAddr;
const prgOffset = effectiveStart - prgBaseAddr;

if (prgOffset < 0 || prgOffset >= rom.prgSize) {
    console.error(`Address ${hex16(effectiveStart)} is outside PRG range ${hex16(prgBaseAddr)}-${hex16(prgBaseAddr + rom.prgSize - 1)}`);
    process.exit(1);
}

console.log(`=== Disassembly from ${hex16(effectiveStart)} ===`);
console.log('');

const { lines, ramAccesses } = disassemble(
    rom.prg.subarray(prgOffset),
    effectiveStart,
    { count, search, ramRefs: showRamRefs }
);

if (!showRamRefs) {
    for (const line of lines) {
        console.log(line);
    }
}

// RAM references report
if (showRamRefs || showSMB) {
    // Full disassembly to collect all references
    const full = disassemble(rom.prg, prgBaseAddr, { ramRefs: true });

    console.log('');
    console.log('=== RAM Address References ($0000-$07FF) ===');
    console.log('');

    const sorted = [...full.ramAccesses.entries()].sort((a, b) => a[0] - b[0]);
    for (const [addr, refs] of sorted) {
        const label = SMB_RAM_LABELS[addr] || '';
        const writes = refs.filter(r => ['STA','STX','STY','INC','DEC'].includes(r.mnemonic));
        const reads = refs.filter(r => ['LDA','LDX','LDY','CMP','CPX','CPY','BIT','AND','ORA','EOR','ADC','SBC'].includes(r.mnemonic));

        if (showSMB && !label) continue; // only show annotated addresses

        console.log(`${hex16(addr)} ${label ? '(' + label + ')' : ''}`);
        console.log(`  Reads: ${reads.length}  Writes: ${writes.length}  Total refs: ${refs.length}`);
        if (refs.length <= 8) {
            for (const ref of refs) {
                console.log(`    ${hex16(ref.pc)}  ${ref.mnemonic} ${ref.operand}`);
            }
        } else {
            for (const ref of refs.slice(0, 4)) {
                console.log(`    ${hex16(ref.pc)}  ${ref.mnemonic} ${ref.operand}`);
            }
            console.log(`    ... and ${refs.length - 4} more`);
        }
        console.log('');
    }
}
