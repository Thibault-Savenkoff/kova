// Runs a module of `kova build --target web -o x.wasm` under node, as a page would (tests/kova.rs):
// node tests/web.mjs x.wasm. Environment variables reach the program; stdin is read by `read_line`,
// each time after a turn of the event loop, as a page's input comes.
import { readFileSync, writeSync, readSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const kovaRun = new Function(readFileSync(join(here, "../runtime/web.js"), "utf8") + "return kovaRun;")();
let ready = false;
const code = await kovaRun(readFileSync(process.argv[2]), {
    write: (fd, bytes) => writeSync(fd, bytes),
    read(into) {
        if (!ready) return -1;
        ready = false;
        try {
            return readSync(0, into);
        } catch {
            return 0;
        }
    },
    input: () =>
        new Promise(done =>
            setTimeout(() => {
                ready = true;
                done();
            }),
        ),
    file(path) {
        try {
            return readFileSync(path);
        } catch (e) {
            return e.code === "EISDIR" ? null : undefined;
        }
    },
    variables: { ...process.env, language: "en" },
});
process.exitCode = code;
