// What a Kova page's module asks of the browser (runtime/web.rs). `host` gives `write(fd, bytes)`,
// `variables`, `read(into)` (some bytes, 0 at the end, -1 if none yet) and `input()` (a promise of
// some): the same code runs in a page and in the tests.
async function kovaRun(module, host) {
    class Stop {
        constructor(code) {
            this.code = code;
        }
    }
    let memory;
    let waiting = null; // what the program left for (runtime/web.rs `leave`)
    const bytes = (at, n) => new Uint8Array(memory.buffer, at, n);
    const imports = {
        kova: {
            write: (fd, at, n) => host.write(fd, bytes(at, n).slice()),
            read: (at, n) => (host.read ? host.read(bytes(at, n)) : 0),
            nap(s) {
                waiting = new Promise(done => setTimeout(done, s * 1000));
            },
            await_input() {
                waiting = host.input();
            },
            stop(code) {
                throw new Stop(code);
            },
            time: () => Date.now(),
            chance(at, n) {
                for (let i = 0; i < n; i += 65536) crypto.getRandomValues(bytes(at + i, Math.min(n - i, 65536)));
            },
            variable(name, length, at, n) {
                const value = host.variables?.[new TextDecoder().decode(bytes(name, length))];
                if (value === undefined) return -1;
                const encoded = new TextEncoder().encode(value);
                if (encoded.length <= n) bytes(at, n).set(encoded);
                return encoded.length;
            },
        },
    };
    const { instance } = await WebAssembly.instantiate(module, imports);
    memory = instance.exports.memory;
    const stopping = run => {
        try {
            run();
        } catch (e) {
            if (e instanceof Stop) return e.code;
            if (e instanceof RangeError) return stopping(instance.exports.kova_web_stack_full);
            throw e;
        }
    };
    for (;;) {
        const code = stopping(instance.exports.kova_start);
        if (code !== undefined) return code;
        await waiting;
    }
}
