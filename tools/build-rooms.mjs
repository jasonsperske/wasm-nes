#!/usr/bin/env node
/**
 * For each subfolder of config/, build dist/<hash>/ containing:
 *   - index.html (copy of frontend/index.html)
 *   - rom-config.json (copy of config/<hash>/rom-config.json)
 *   - <hash>.nes (copy of rooms/<hash>.nes)
 */

import { readdirSync, statSync, mkdirSync, copyFileSync, existsSync } from 'fs';
import { fileURLToPath } from 'url';
import { dirname, resolve } from 'path';

const __dirname = dirname(fileURLToPath(import.meta.url));
const root = resolve(__dirname, '..');
const configDir = resolve(root, 'config');
const roomsDir = resolve(root, 'rooms');
const distDir = resolve(root, 'dist');
const srcIndex = resolve(root, 'frontend', 'index.html');

if (!existsSync(distDir)) mkdirSync(distDir, { recursive: true });

const hashes = readdirSync(configDir).filter(n => statSync(resolve(configDir, n)).isDirectory());

for (const hash of hashes) {
    const outDir = resolve(distDir, hash);
    mkdirSync(outDir, { recursive: true });

    copyFileSync(srcIndex, resolve(outDir, 'index.html'));

    const cfg = resolve(configDir, hash, 'rom-config.json');
    if (existsSync(cfg)) {
        copyFileSync(cfg, resolve(outDir, 'rom-config.json'));
    } else {
        console.warn(`[build-rooms] missing rom-config.json for ${hash}`);
    }

    const rom = resolve(roomsDir, `${hash}.nes`);
    if (existsSync(rom)) {
        copyFileSync(rom, resolve(outDir, `${hash}.nes`));
    } else {
        console.warn(`[build-rooms] missing rooms/${hash}.nes — skipping`);
    }

    console.log(`[build-rooms] dist/${hash}/`);
}
