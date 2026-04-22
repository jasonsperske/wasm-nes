/**
 * Pull-based audio using an AudioWorklet + ring buffer.
 *
 * - Main thread pushes sample chunks via port.postMessage (transferable).
 * - Worklet processor pulls at exact sampleRate on the audio thread,
 *   decoupling playback from RAF jitter and display-vs-NTSC clock drift.
 * - Ring buffer absorbs both production spikes (dropping oldest on overflow)
 *   and brief production stalls (outputting silence on underrun).
 */

// Source for the AudioWorklet. Lives here as a string so it can be loaded
// via a Blob URL without shipping a separate file or tweaking the bundler.
const WORKLET_SRC = `
class NesAudioProcessor extends AudioWorkletProcessor {
    constructor () {
        super();
        // 500 ms ring buffer at the context's sample rate.
        this.capacity = Math.floor(sampleRate * 0.5);
        this.buffer = new Float32Array(this.capacity);
        this.readPos = 0;
        this.writePos = 0;
        this.size = 0;

        this.underruns = 0;
        this.overflows = 0;
        this.totalConsumed = 0;
        this.processCalls = 0;

        this.port.onmessage = (e) => {
            const chunk = e.data;
            if (!(chunk instanceof Float32Array)) return;
            for (let i = 0; i < chunk.length; i++) {
                if (this.size < this.capacity) {
                    this.buffer[this.writePos] = chunk[i];
                    this.writePos = (this.writePos + 1) % this.capacity;
                    this.size++;
                } else {
                    // Overflow: drop oldest sample to make room.
                    this.buffer[this.writePos] = chunk[i];
                    this.writePos = (this.writePos + 1) % this.capacity;
                    this.readPos = (this.readPos + 1) % this.capacity;
                    this.overflows++;
                }
            }
        };
    }

    process (inputs, outputs) {
        const out = outputs[0][0];
        for (let i = 0; i < out.length; i++) {
            if (this.size > 0) {
                out[i] = this.buffer[this.readPos];
                this.readPos = (this.readPos + 1) % this.capacity;
                this.size--;
                this.totalConsumed++;
            } else {
                out[i] = 0;
                this.underruns++;
            }
        }

        this.processCalls++;
        // Report stats ~every 100 ms (one process block = 128 samples ≈ 2.67 ms @ 48 kHz).
        if (this.processCalls >= 40) {
            this.processCalls = 0;
            this.port.postMessage({
                type: 'stats',
                size: this.size,
                capacity: this.capacity,
                underruns: this.underruns,
                overflows: this.overflows,
                totalConsumed: this.totalConsumed,
            });
        }
        return true;
    }
}
registerProcessor('nes-audio', NesAudioProcessor);
`;

export interface AudioStats {
    sampleRate: number;
    baseLatency: number;
    bufferedMs: number;
    bufferCapacityMs: number;
    underruns: number;
    overflows: number;
    chunksQueued: number;
    lastChunkSamples: number;
    lastChunkMs: number;
    totalSamplesQueued: number;
    totalSamplesConsumed: number;
    contextState: AudioContextState;
}

interface WorkletStats {
    size: number;
    capacity: number;
    underruns: number;
    overflows: number;
    totalConsumed: number;
}

export class Audio {
    #context: AudioContext;
    #gain: GainNode;
    #analyzer: AnalyserNode;
    #worklet: AudioWorkletNode | null;
    #workletReady: Promise<void> | null;
    #pending: Float32Array[];
    #chunksQueued: number;
    #lastChunkSamples: number;
    #totalSamplesQueued: number;
    #workletStats: WorkletStats;
    data: {
        timeDomain: Uint8Array,
        frequency: Uint8Array,
    };

    constructor () {
        this.#context = new AudioContext();
        this.#gain = this.#context.createGain();
        this.#gain.gain.value = 1;
        this.#analyzer = this.#context.createAnalyser();
        this.#analyzer.minDecibels = -100;
        this.#analyzer.maxDecibels = 0;
        this.#analyzer.smoothingTimeConstant = 0;
        this.#worklet = null;
        this.#workletReady = null;
        this.#pending = [];
        this.#chunksQueued = 0;
        this.#lastChunkSamples = 0;
        this.#totalSamplesQueued = 0;
        this.#workletStats = { size: 0, capacity: 0, underruns: 0, overflows: 0, totalConsumed: 0 };
        this.data = {
            timeDomain: new Uint8Array(this.#analyzer.fftSize),
            frequency: new Uint8Array(this.#analyzer.frequencyBinCount),
        };

        this.#gain.connect(this.#analyzer);
    }

    async #ensureWorklet () {
        if (this.#worklet) return;
        if (!this.#workletReady) {
            const blob = new Blob([WORKLET_SRC], { type: 'application/javascript' });
            const url = URL.createObjectURL(blob);
            this.#workletReady = this.#context.audioWorklet.addModule(url).then(() => {
                URL.revokeObjectURL(url);
                const node = new AudioWorkletNode(this.#context, 'nes-audio');
                node.port.onmessage = (e) => {
                    if (e.data?.type === 'stats') {
                        this.#workletStats = e.data as WorkletStats;
                    }
                };
                // Prime the ring with 100 ms of silence BEFORE connecting to the
                // graph, so the worklet has a cushion when it first starts
                // processing. Without this, the worklet consumes samples faster
                // than the first chunk can arrive from RAF, causing an initial
                // underrun plus near-zero steady-state buffer.
                const prerollSamples = Math.floor(this.#context.sampleRate * 0.1);
                const silence = new Float32Array(prerollSamples);
                node.port.postMessage(silence, [silence.buffer]);

                // Flush any chunks queued before the worklet was ready.
                for (const p of this.#pending) {
                    node.port.postMessage(p, [p.buffer]);
                }
                this.#pending = [];

                node.connect(this.#gain);
                this.#worklet = node;
            });
        }
        await this.#workletReady;
    }

    start () {
        this.#chunksQueued = 0;
        this.#lastChunkSamples = 0;
        this.#totalSamplesQueued = 0;
        this.#workletStats = { size: 0, capacity: 0, underruns: 0, overflows: 0, totalConsumed: 0 };
        this.#analyzer.connect(this.#context.destination);
        this.#ensureWorklet().catch(err => console.error('AudioWorklet failed:', err));
    }

    stop () {
        this.#analyzer.disconnect();
    }

    analyze () {
        this.#analyzer.getByteFrequencyData(this.data.frequency);
        this.#analyzer.getByteTimeDomainData(this.data.timeDomain);
    }

    queue (chunk: Float32Array) {
        if (chunk.length === 0) return;
        const len = chunk.length;
        if (this.#worklet) {
            this.#worklet.port.postMessage(chunk, [chunk.buffer]);
        } else {
            // Worklet not ready yet — copy and buffer. (Can't transfer now because
            // the WASM-owned buffer will be needed again by the next get_audio call.)
            this.#pending.push(new Float32Array(chunk));
        }
        this.#chunksQueued++;
        this.#lastChunkSamples = len;
        this.#totalSamplesQueued += len;
    }

    get stats (): AudioStats {
        const sr = this.#context.sampleRate;
        const ws = this.#workletStats;
        return {
            sampleRate: sr,
            baseLatency: this.#context.baseLatency ?? 0,
            bufferedMs: (ws.size / sr) * 1000,
            bufferCapacityMs: (ws.capacity / sr) * 1000,
            underruns: ws.underruns,
            overflows: ws.overflows,
            chunksQueued: this.#chunksQueued,
            lastChunkSamples: this.#lastChunkSamples,
            lastChunkMs: (this.#lastChunkSamples / sr) * 1000,
            totalSamplesQueued: this.#totalSamplesQueued,
            totalSamplesConsumed: ws.totalConsumed,
            contextState: this.#context.state,
        };
    }

    fix () {
        // iOS-specific
        this.#context.resume();
    }

    get sampleRate () {
        return this.#context.sampleRate;
    }

    get time () {
        return this.#context.currentTime;
    }

    set volume (volume: number) {
        this.#gain.gain.value = volume;
    }
}
