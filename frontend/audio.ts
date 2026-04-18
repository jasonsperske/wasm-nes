export class Audio {
    #context: AudioContext;
    #gain: GainNode;
    #analyzer: AnalyserNode;
    #nextStartTime: number;
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
        this.#nextStartTime = 0;
        this.data = {
            timeDomain: new Uint8Array(this.#analyzer.fftSize),
            frequency: new Uint8Array(this.#analyzer.frequencyBinCount),
        };

        this.#gain.connect(this.#analyzer);
    }

    start () {
        this.#nextStartTime = this.#context.currentTime;
        this.#analyzer.connect(this.#context.destination);
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

        const node = this.#context.createBufferSource();
        node.buffer = this.#context.createBuffer(1, chunk.length, this.#context.sampleRate);
        node.buffer.copyToChannel(chunk, 0);
        node.connect(this.#gain);

        // Schedule this chunk right after the previous one ends
        const now = this.#context.currentTime;
        if (this.#nextStartTime < now) {
            this.#nextStartTime = now;
        }
        node.start(this.#nextStartTime);
        this.#nextStartTime += node.buffer.duration;
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
