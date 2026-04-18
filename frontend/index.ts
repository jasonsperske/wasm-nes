import GameStats from 'game-stats';

import wasm from '../backend/pkg/index_bg.wasm';
import init, { Button, Emulator, set_panic_hook } from '../backend/pkg';
import { Debug } from './debug';
import { Logs } from './logs';
import { Audio } from './audio';

export enum Status {
    IDLE,
    RUNNING,
    ERROR,
}

export interface MemoryWatch {
    address: number;
    name: string;
}

export interface MemoryChangeEvent {
    address: number;
    name: string;
    oldValue: number;
    newValue: number;
}

export class Nes {
    static VIDEO_WIDTH = 256;
    static VIDEO_HEIGHT = 240;

    canvas: HTMLCanvasElement;
    error: Error;
    logs: Logs;
    memory: WebAssembly.Memory;
    debug: Debug;
    audio: Audio;
    onCycle?: () => void;
    onStatus?: () => void;
    onMemoryChange?: (changes: MemoryChangeEvent[]) => void;
    onScreenChange?: (event: { type: string, values: Record<string, number> }) => void;

    #vm: Emulator;
    #rafHandle: ReturnType<typeof requestAnimationFrame>;
    #stats: GameStats;
    #watches: MemoryWatch[];
    #watchValues: Map<number, number>;

    static async new (rom) {
        const { memory } = await init({ module_or_path: wasm });
        return new Nes(rom, memory);
    }

    private constructor (rom, memory) {
        this.logs = new Logs();
        this.memory = memory;
        this.audio = new Audio();
        this.#vm = Emulator.new(rom, this.audio.sampleRate);
        this.#stats = new GameStats({ historyLimit: 100 });
        this.#watches = [];
        this.#watchValues = new Map();

        set_panic_hook((message) => this.stop(new Error(message)));
        // db.getAll().then(setSaves).catch(setError);
    }

    start () {
        const rafCallback = (timestamp) => {
            this.cycleUntil('frame');
            this.#stats.record(timestamp);
            // Don't run another frame if it has been canceled in the mean time
            if (this.#rafHandle) {
                this.#rafHandle = requestAnimationFrame(rafCallback);
            }
        };

        this.#rafHandle = requestAnimationFrame(rafCallback);
        this.audio.start();
        this.onStatus?.();
    }

    stop (error?: Error) {
        this.audio.stop();
        cancelAnimationFrame(this.#rafHandle);
        this.#rafHandle = null;

        if (error && error instanceof Error) {
            console.error(error);
            this.error = error;
        }

        this.onStatus?.();
    }

    reset () {
        this.#vm.reset();
    }

    saveState (): Uint8Array {
        return this.#vm.save_state();
    }

    loadState (data: Uint8Array) {
        this.#vm.load_state(data);
    }

    readMemory (address: number): number {
        return this.#vm.read(address);
    }

    watchMemory (watches: MemoryWatch[]) {
        this.#watches = watches;
        this.#watchValues.clear();
        // Snapshot current values
        for (const w of watches) {
            this.#watchValues.set(w.address, this.#vm.read(w.address));
        }
    }

    private pollWatches () {
        if (this.#watches.length === 0) return;

        const changes: MemoryChangeEvent[] = [];
        for (const w of this.#watches) {
            const newValue = this.#vm.read(w.address);
            const oldValue = this.#watchValues.get(w.address)!;
            if (newValue !== oldValue) {
                changes.push({ address: w.address, name: w.name, oldValue, newValue });
                this.#watchValues.set(w.address, newValue);
            }
        }

        if (changes.length > 0) {
            this.onMemoryChange?.(changes);

            // Fire onScreenChange for screen-transition-related watches
            if (this.onScreenChange) {
                const screenKeys = ['gameMode', 'playerState', 'areaType', 'screenPage',
                                     'currentWorld', 'currentLevel', 'warpZone',
                                     'altEntrance', 'areaStyle', 'screenRoutine'];
                const isScreenChange = changes.some(c => screenKeys.includes(c.name));
                if (isScreenChange) {
                    const values: Record<string, number> = {};
                    for (const w of this.#watches) {
                        values[w.name] = this.#watchValues.get(w.address)!;
                    }
                    const trigger = changes.find(c => screenKeys.includes(c.name))!;
                    this.onScreenChange({ type: trigger.name, values });
                }
            }
        }
    }

    private cycle (fn) {
        try {
            // this.vm.update_controllers(this.inputs);
            fn();
            this.debug = new Debug(this.#vm);
            this.render();
            this.audio.analyze();
            this.audio.queue(this.#vm.get_audio());
            this.pollWatches();
        } catch (err) {
            // Don't call stop() here, because the original error will already be caught by the panic hook
            console.error(err);
        } finally {
            this.onCycle?.();
        }
    }

    cycleUntil (duration) {
        switch (duration) {
            case 'tick': this.cycle(this.#vm.cycle.bind(this.#vm)); break;
            case 'cpu': this.cycle(this.#vm.cycle_until_cpu.bind(this.#vm)); break;
            case 'ppu': this.cycle(this.#vm.cycle_until_ppu.bind(this.#vm)); break;
            case 'scanline': this.cycle(this.#vm.cycle_until_scanline.bind(this.#vm)); break;
            case 'frame': this.cycle(this.#vm.cycle_until_frame.bind(this.#vm)); break;
            default: console.warn('Unknown cycle duration');
        }
    }

    private render () {
        this.canvas?.getContext('2d').putImageData(new ImageData(this.#vm.get_framebuffer(), Nes.VIDEO_WIDTH, Nes.VIDEO_HEIGHT), 0, 0);
    }

    input (player: number, button: Button, pressed: boolean) {
        this.#vm.update_controller(player, button, pressed);
    }

    get status () {
        if (this.error) {
            return Status.ERROR;
        } else if (this.#rafHandle) {
            return Status.RUNNING;
        } else {
            return Status.IDLE;
        }
    }

    get performance () {
        return this.#stats.stats();
    }
}

export type { MemoryWatch, MemoryChangeEvent };

export {
    Button,
    CpuStatusFlag,
    PpuCtrlFlag,
    PpuMaskFlag,
    PpuStatusFlag,
    SpriteAttribute,
} from '../backend/pkg';
