#!/usr/bin/env node
/**
 * Generate a rom-config.json by analyzing a ROM with the disassembler
 * and sending the analysis to Claude for interpretation.
 *
 * Usage:
 *   node tools/generate-config.mjs rom.nes                    # Output prompt to stdout (pipe to Claude)
 *   node tools/generate-config.mjs rom.nes --call              # Call Claude API directly (needs ANTHROPIC_API_KEY)
 *   node tools/generate-config.mjs rom.nes --call -o config.json  # Call API and write to file
 */

import { readFileSync, writeFileSync } from 'fs';
import { execSync } from 'child_process';
import { fileURLToPath } from 'url';
import { dirname, resolve } from 'path';

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);
const disasmPath = resolve(__dirname, 'disasm.mjs');

const args = process.argv.slice(2);
if (args.length === 0) {
    console.log('Usage: node tools/generate-config.mjs <rom.nes> [--call] [-o output.json]');
    console.log('');
    console.log('  Without --call: prints a prompt to stdout. Paste into Claude to get config.');
    console.log('  With --call:    calls the Claude API directly (requires ANTHROPIC_API_KEY env var).');
    console.log('  With -o:        writes the result to a file instead of stdout.');
    process.exit(0);
}

const romPath = args[0];
const shouldCall = args.includes('--call');
const outIdx = args.indexOf('-o');
const outPath = outIdx >= 0 ? args[outIdx + 1] : null;

// Run disassembler to get header + vectors + RAM refs
const header = execSync(`node "${disasmPath}" "${romPath}" --header --vectors`, { encoding: 'utf-8' });
const ramRefs = execSync(`node "${disasmPath}" "${romPath}" --ram-refs`, { encoding: 'utf-8' });

// Get first 100 instructions from reset vector for game identification
const resetCode = execSync(`node "${disasmPath}" "${romPath}" --vectors --count 100`, { encoding: 'utf-8' });

const configSchema = JSON.stringify({
    name: "Game Title",
    mapper: 0,
    watches: [
        { address: "0xNNNN", name: "camelCaseName", description: "What this address tracks" }
    ],
    events: [
        {
            name: "eventName",
            label: "Human readable event description",
            trigger: { watch: "watchName", equals: 0 },
            saveState: {
                delayFrames: 36,
                keyTemplate: "prefix-{token1}-{token2}",
                reads: [
                    { address: "0xNNNN", token: "token1" }
                ]
            }
        }
    ]
}, null, 2);

const prompt = `You are analyzing a disassembled NES ROM to generate a rom-config.json file for an emulator editor tool.

The config defines RAM addresses to watch during gameplay and events that trigger save states (e.g., when the player enters a pipe, door, or transitions between screens).

## ROM Header & Vectors
\`\`\`
${header}
\`\`\`

## First 100 Instructions from Reset Vector
\`\`\`
${resetCode}
\`\`\`

## All RAM Address References ($0000-$07FF)
This shows every RAM address the game reads/writes, with reference counts and sample instructions:
\`\`\`
${ramRefs}
\`\`\`

## Your Task

1. **Identify the game** from the code patterns, mapper number, and ROM structure.
2. **Select the most important RAM addresses** to watch — focus on:
   - Player state (alive, dead, transitioning, entering pipe/door)
   - Game mode (title screen, playing, paused, game over)
   - World/level/stage/room identifiers
   - Area type (overworld, underground, underwater, castle, etc.)
   - Screen position or scroll state
   - Any addresses related to screen transitions, warp zones, or door entry
3. **Define events** that detect screen transitions (pipes, doors, stairs, warp zones, level transitions). For each event:
   - Which watch to monitor and what value triggers it
   - A delay in frames before saving (typically 30-60 frames to let the transition animate)
   - A key template using tokens read from RAM at save time
   - The RAM addresses to read for each token

## Output Format

Return ONLY valid JSON matching this schema (no markdown fences, no explanation):
${configSchema}

Guidelines:
- Use hex strings for addresses (e.g., "0x000E")
- Use camelCase for watch names and event names
- Include 5-10 watches covering the most important game state
- Include at least one event for screen transitions
- The keyTemplate should produce unique, human-readable save state names
- delayFrames should be 30-60 (0.5-1 second at 60fps)
- Only include events you're confident about based on the RAM analysis`;

if (!shouldCall) {
    if (outPath) {
        writeFileSync(outPath, prompt);
        console.log(`Prompt written to ${outPath}`);
    } else {
        console.log(prompt);
    }
    console.error('\n---');
    console.error('Paste the above into Claude to generate rom-config.json.');
    console.error('Or use --call flag to call the Claude API directly.');
    process.exit(0);
}

// Call Claude API
const apiKey = process.env.ANTHROPIC_API_KEY;
if (!apiKey) {
    console.error('Error: ANTHROPIC_API_KEY environment variable is required with --call flag.');
    process.exit(1);
}

console.error('Calling Claude API...');

const response = await fetch('https://api.anthropic.com/v1/messages', {
    method: 'POST',
    headers: {
        'Content-Type': 'application/json',
        'x-api-key': apiKey,
        'anthropic-version': '2023-06-01',
    },
    body: JSON.stringify({
        model: 'claude-sonnet-4-20250514',
        max_tokens: 4096,
        messages: [{ role: 'user', content: prompt }],
    }),
});

if (!response.ok) {
    const err = await response.text();
    console.error(`API error ${response.status}: ${err}`);
    process.exit(1);
}

const result = await response.json();
const text = result.content[0].text.trim();

// Extract JSON (handle if Claude wraps it in markdown fences)
let json = text;
const fenceMatch = text.match(/```(?:json)?\s*\n([\s\S]*?)\n```/);
if (fenceMatch) json = fenceMatch[1].trim();

// Validate it parses
try {
    JSON.parse(json);
} catch (e) {
    console.error('Warning: Claude returned invalid JSON. Raw output:');
    console.error(text);
    process.exit(1);
}

if (outPath) {
    writeFileSync(outPath, json + '\n');
    console.error(`Config written to ${outPath}`);
} else {
    console.log(json);
}
