// What a Kova page's module asks of the browser (runtime/web.rs). `host` gives `write(fd, bytes)`,
// `variables`, `read(into)` (some bytes, 0 at the end, -1 if none yet) and `input()` (a promise of
// some): the same code runs in a page and in the tests.
async function kovaRun(module, host) {
    class Stop {
        constructor(code) {
            this.code = code;
        }
    }
    const text = (at, n) => new TextDecoder().decode(bytes(at, n));
    const page = kovaPage();
    let memory;
    let incoming = null; // bytes on their way into the module
    let fetchState = -1;
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
            ui_render(at, n) {
                const failed = page.render(JSON.parse(text(at, n)));
                waiting = page.next();
                return failed;
            },
            ui_clicked: () => page.take(),
            ui_typed(at, n) {
                const encoded = new TextEncoder().encode(page.typed);
                if (encoded.length <= n) bytes(at, n).set(encoded);
                return encoded.length;
            },
            ui_rooms: () => page.rooms.length,
            ui_room: (k, which) => page.rooms[k][which],
            ui_area: k => page.area(k),
            ui_ask(kind, title, n, body, m) {
                return page.ask(kind, text(title, n), text(body, m)) ? 1 : 0;
            },
            file_size(at, n) {
                const data = host.file?.(text(at, n));
                if (data === undefined) return -1;
                if (data === null) return -2;
                incoming = data;
                return data.length;
            },
            file_read(at) {
                bytes(at, incoming.length).set(incoming);
            },
            pick_file() {
                waiting = page.pick();
            },
            picked() {
                if (!page.chosen) return -1;
                incoming = page.chosen.data;
                page.typed = page.chosen.name;
                queueMicrotask(() => (page.chosen = null));
                return incoming.length;
            },
            download(name, n, at, m) {
                page.download(text(name, n), bytes(at, m).slice());
            },
            fetch_start(json, n, at, m) {
                const asked = JSON.parse(text(json, n));
                const body = m ? bytes(at, m).slice() : undefined;
                waiting = (async () => {
                    try {
                        const answer = await fetch(asked.url, {
                            method: asked.method,
                            headers: asked.headers,
                            body,
                            redirect: asked.follow ? "follow" : "manual",
                            signal: asked.ms ? AbortSignal.timeout(asked.ms) : undefined,
                        });
                        const data = new Uint8Array(await answer.arrayBuffer());
                        const lines = [...answer.headers].map(([name, value]) => `${name}: ${value}\n`).join("");
                        const head = new TextEncoder().encode(`${answer.status}\n${answer.url}\n${lines}\n`);
                        incoming = new Uint8Array(head.length + data.length);
                        incoming.set(head);
                        incoming.set(data, head.length);
                        fetchState = incoming.length;
                    } catch (e) {
                        fetchState = e.name === "TimeoutError" ? -2 : -1;
                    }
                })();
            },
            fetched: () => fetchState,
            play(at, n) {
                waiting = page.play(bytes(at, n).slice());
            },
            variable(name, length, at, n) {
                const value = host.variables?.[text(name, length)];
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

// A view of `import ui` as elements of the page. `render` keeps the elements while the view's
// shape stays (a field keeps its cursor), and only changes what differs.
function kovaPage() {
    const web = typeof document !== "undefined";
    const RESIZED = 2 ** 40;
    let root = null;
    let shape = null;
    let wake = null;
    let closed = false;
    let failed = 0;
    const queue = [];
    const clocks = new Map();
    const images = new Map();
    const self = { rooms: [], typed: "" };

    // As `shared::UiQueue`: a slow program keeps the last move and one tick; a press is never lost.
    const hear = (answer, words = "") => {
        if (words === "" && queue.some(w => w[0] === answer)) return;
        const last = queue[queue.length - 1];
        if (last && last[0] === answer && last[1].startsWith("move ") && words.startsWith("move ")) last[1] = words;
        else queue.push([answer, words]);
        if (wake) {
            wake();
            wake = null;
        }
    };
    self.next = () => (queue.length ? Promise.resolve() : new Promise(done => (wake = done)));
    self.take = () => {
        const [answer, words] = queue.shift();
        self.typed = words;
        return answer;
    };
    const changed = k => -2 - k;
    const color = c => "#" + c.toString(16).padStart(6, "0");

    function style(el, s) {
        el.style.color = s.color === null || s.color === undefined ? "" : color(s.color);
        el.style.fontSize = s.size ? s.size + "px" : "";
        el.style.fontWeight = s.bold ? "bold" : "";
    }

    // Each kind of view: `make` builds the element, `change` brings it up to date.
    const kinds = {
        T: {
            make: () => document.createElement("div"),
            change(el, [, text], s) {
                if (el.textContent !== text) el.textContent = text;
                style(el, s);
            },
        },
        B: {
            make(n) {
                const el = document.createElement("button");
                el.onclick = () => hear(el.number);
                el.number = n[2];
                return el;
            },
            change(el, [, label, k], s) {
                el.number = k;
                if (el.textContent !== label) el.textContent = label;
                style(el, { ...s, color: null });
            },
        },
        I: {
            make() {
                const el = document.createElement("input");
                el.oninput = () => hear(changed(el.number), el.value);
                return el;
            },
            change(el, [, value, k], s) {
                el.number = k;
                if (el.value !== value) el.value = value;
                style(el, s);
            },
        },
        X: {
            make() {
                const el = document.createElement("label");
                el.box = el.appendChild(document.createElement("input"));
                el.box.type = "checkbox";
                el.words = el.appendChild(document.createElement("span"));
                el.box.onchange = () => hear(changed(el.number), el.box.checked ? "on" : "off");
                return el;
            },
            change(el, [, label, on, k]) {
                el.number = k;
                el.words.textContent = " " + label;
                el.box.checked = on;
            },
        },
        W: {
            make() {
                const el = document.createElement("input");
                el.type = "range";
                el.oninput = () => hear(changed(el.number), el.value);
                return el;
            },
            change(el, [, value, min, max, k]) {
                el.number = k;
                el.min = min;
                el.max = max;
                if (el.value != value) el.value = value;
            },
        },
        O: {
            make(n) {
                const el = document.createElement("select");
                for (const item of n[1]) el.add(new Option(item));
                el.onchange = () => hear(changed(el.number), String(el.selectedIndex));
                return el;
            },
            change(el, [, , selected, k]) {
                el.number = k;
                el.selectedIndex = selected;
            },
        },
        G: {
            make(n) {
                const el = document.createElement("table");
                el.className = "kova-table";
                if (n[1].length) {
                    const row = el.createTHead().insertRow();
                    for (const h of n[1]) row.appendChild(document.createElement("th")).textContent = h;
                }
                const body = el.createTBody();
                n[2].forEach((cells, i) => {
                    const row = body.insertRow();
                    row.onclick = () => hear(changed(el.number), String(i));
                    for (const c of cells) row.insertCell().textContent = c;
                });
                return el;
            },
            change(el, [, , , selected, k]) {
                el.number = k;
                [...el.tBodies[0].rows].forEach((r, i) => r.classList.toggle("kova-chosen", i === selected));
            },
        },
        P: {
            make: () => document.createElement("img"),
            change(el, [, path, width]) {
                if (el.getAttribute("src") !== path) el.src = path;
                if (width) el.width = width;
            },
        },
        C: group("column"),
        R: group("row"),
        M: {
            make(n, s) {
                const el = document.createElement("div");
                el.appendChild(build(n[2], s));
                return el;
            },
            change(el, n, s) {
                el.style.padding = n[1] + "px";
                update(el.firstChild, n[2], s);
            },
        },
        S: {
            make: (n, s) => build(n[4], over(n, s)),
            change: (el, n, s) => update(el, n[4], over(n, s)),
        },
        E: {
            make: (n, s) => build(n[3], s),
            change: (el, n, s) => update(el, n[3], s),
        },
        D: {
            make(n) {
                const el = document.createElement("canvas");
                el.tabIndex = 0;
                const where = e => {
                    const box = el.getBoundingClientRect();
                    return `${Math.round(e.clientX - box.left)} ${Math.round(e.clientY - box.top)}`;
                };
                const mouse = (kind, e) => {
                    if (el.mouse === null) return;
                    hear(changed(el.mouse), `${kind} ${where(e)} ${kind === "move" ? 0 : e.button === 2 ? 2 : 1}`);
                };
                el.onmousedown = e => mouse("press", e);
                el.onmouseup = e => mouse("release", e);
                el.onmousemove = e => mouse("move", e);
                el.oncontextmenu = e => e.preventDefault();
                el.onkeydown = e => {
                    if (el.key === null || ["Shift", "Control", "Alt", "Meta"].includes(e.key)) return;
                    e.preventDefault();
                    hear(changed(el.key), "key " + (e.key === " " ? "Space" : e.key.replace(/^Arrow/, "")));
                };
                return el;
            },
            change: paint,
        },
    };

    function over(n, s) {
        return { color: n[1] ?? s.color, size: n[2] || s.size, bold: n[3] || s.bold };
    }

    function group(kind) {
        return {
            make(n, s) {
                const el = document.createElement("div");
                el.className = "kova-" + kind;
                for (const part of n[2]) el.appendChild(build(part, s));
                return el;
            },
            change(el, n, s) {
                el.style.alignItems = ["flex-start", "center", "flex-end"][n[1]];
                el.classList.toggle("kova-placing", n[1] !== 0);
                n[2].forEach((part, i) => update(el.children[i], part, s));
            },
        };
    }

    const build = (n, s) => {
        const el = kinds[n[0]].make(n, s);
        kinds[n[0]].change(el, n, s);
        return el;
    };
    const update = (el, n, s) => kinds[n[0]].change(el, n, s);

    const picture = path => {
        if (!images.has(path)) {
            const image = new Image();
            image.onload = () => hear(RESIZED);
            image.src = path;
            images.set(path, image);
        }
        return images.get(path);
    };

    // A canvas: `[D, width, height, number, mouse, key, ops]`; 0 = all the room there is.
    function paint(el, [, width, height, number, mouseAt, keyAt, ops]) {
        el.mouse = mouseAt;
        el.key = keyAt;
        el.classList.toggle("kova-fill-width", width === 0);
        el.classList.toggle("kova-fill-height", height === 0);
        if (width) el.style.width = width + "px";
        if (height) el.style.height = height + "px";
        const box = el.getBoundingClientRect();
        const [w, h] = [Math.round(box.width) || 320, Math.round(box.height) || 200];
        const was = self.rooms[number];
        self.rooms[number] = [w, h];
        if (was && (was[0] !== w || was[1] !== h || !el.painted)) hear(RESIZED);
        if (!was) hear(RESIZED);
        el.painted = true;
        const dpr = globalThis.devicePixelRatio || 1;
        const size = [Math.round(w * dpr), Math.round(h * dpr)];
        if (el.width !== size[0] || el.height !== size[1]) [el.width, el.height] = size;
        if (ops[0]?.[0] === "g") return gpu(el, ops, size);
        const g = el.getContext("2d");
        g.setTransform(dpr, 0, 0, dpr, 0, 0);
        g.textBaseline = "top";
        for (const op of ops) {
            const [kind] = op;
            if (kind === "C") {
                g.fillStyle = color(op[1]);
                g.fillRect(0, 0, w, h);
            } else if (kind === "R") {
                const [, x, y, rw, rh, c, outline] = op;
                g.fillStyle = g.strokeStyle = color(c);
                g.lineWidth = outline;
                if (outline) g.strokeRect(x, y, rw, rh);
                else g.fillRect(x, y, rw, rh);
            } else if (kind === "O") {
                const [, x, y, r, c, outline] = op;
                g.fillStyle = g.strokeStyle = color(c);
                g.lineWidth = outline;
                g.beginPath();
                g.arc(x, y, r, 0, 2 * Math.PI);
                if (outline) g.stroke();
                else g.fill();
            } else if (kind === "L") {
                const [, x, y, x2, y2, c, thickness] = op;
                g.strokeStyle = color(c);
                g.lineWidth = thickness;
                g.beginPath();
                g.moveTo(x, y);
                g.lineTo(x2, y2);
                g.stroke();
            } else if (kind === "T") {
                const [, x, y, size, c, words] = op;
                g.fillStyle = color(c);
                g.font = size + "px sans-serif";
                g.fillText(words, x, y);
            } else if (kind === "I") {
                const [, x, y, iw, ih, path] = op;
                const image = picture(path);
                if (image.complete && image.naturalWidth) g.drawImage(image, x, y, iw || image.naturalWidth, ih || image.naturalHeight);
            }
        }
    }

    // `ui.gpu`: the same steps as `gl_paint` in runtime/kova.rs, on WebGL 2. The shaders are written
    // for OpenGL 3.3; only their first line differs.
    function gpu(el, ops, [w, h]) {
        const gl = el.gl ?? (el.gl = el.getContext("webgl2", { preserveDrawingBuffer: true }));
        if (!gl) {
            failed = 1;
            return;
        }
        if (!el.state) el.state = { array: gl.createVertexArray(), buffer: gl.createBuffer(), programs: new Map() };
        const state = el.state;
        gl.bindVertexArray(state.array);
        gl.bindBuffer(gl.ARRAY_BUFFER, state.buffer);
        gl.viewport(0, 0, w, h);
        gl.enable(gl.DEPTH_TEST);
        gl.depthFunc(gl.LEQUAL);
        gl.enable(gl.BLEND);
        gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
        const wipe = c => {
            gl.clearColor(((c >> 16) & 255) / 255, ((c >> 8) & 255) / 255, (c & 255) / 255, 1);
            gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
        };
        wipe(0);
        const compile = (kind, source) => {
            const shader = gl.createShader(kind);
            gl.shaderSource(shader, source.replace(/^\s*#version.*/, "#version 300 es\nprecision highp float;"));
            gl.compileShader(shader);
            if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
                failed = 2;
                queue.unshift([0, gl.getShaderInfoLog(shader)]);
            }
            return shader;
        };
        let program = null;
        const textures = [];
        for (const op of ops) {
            if (op[0] === "c") wipe(op[1]);
            else if (op[0] === "s") {
                const key = op[1] + "\0" + op[2];
                if (!state.programs.has(key)) {
                    const made = gl.createProgram();
                    gl.attachShader(made, compile(gl.VERTEX_SHADER, op[1]));
                    gl.attachShader(made, compile(gl.FRAGMENT_SHADER, op[2]));
                    gl.linkProgram(made);
                    state.programs.set(key, made);
                }
                program = state.programs.get(key);
                gl.useProgram(program);
            } else if (op[0] === "u") {
                const at = gl.getUniformLocation(program, op[1]);
                const v = new Float32Array(op[2]);
                if (v.length === 16) gl.uniformMatrix4fv(at, false, v);
                else if (v.length >= 1 && v.length <= 4) gl["uniform" + v.length + "fv"](at, v);
            } else if (op[0] === "v") {
                const sizes = op[1];
                const one = sizes.reduce((a, b) => a + b, 0);
                if (!one) continue;
                gl.bufferData(gl.ARRAY_BUFFER, new Float32Array(op[2]), gl.STREAM_DRAW);
                let at = 0;
                sizes.forEach((size, k) => {
                    gl.enableVertexAttribArray(k);
                    gl.vertexAttribPointer(k, size, gl.FLOAT, false, one * 4, at * 4);
                    at += size;
                });
                gl.drawArrays(gl.TRIANGLES, 0, op[2].length / one);
            } else if (op[0] === "x") {
                const texture = gl.createTexture();
                gl.bindTexture(gl.TEXTURE_2D, texture);
                gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
                gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
                gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, op[1], op[2], 0, gl.RGBA, gl.UNSIGNED_BYTE, new Uint8Array(op[3]));
                textures.push(texture);
            }
        }
        textures.forEach(t => gl.deleteTexture(t));
    }

    function menus(list) {
        const bar = document.getElementById("kova-menus");
        bar.replaceChildren();
        for (const menu of list) {
            const box = bar.appendChild(document.createElement("details"));
            box.appendChild(document.createElement("summary")).textContent = menu.title;
            const items = box.appendChild(document.createElement("div"));
            for (const [label, k] of menu.items) {
                if (k === null) {
                    items.appendChild(document.createElement("hr"));
                    continue;
                }
                const item = items.appendChild(document.createElement("button"));
                item.textContent = label;
                item.onclick = () => {
                    box.open = false;
                    hear(k);
                };
            }
        }
    }

    function clocksOf(n, found) {
        if (n[0] === "E") found.push([n[1], n[2]]);
        if (n[0] === "C" || n[0] === "R") n[2].forEach(p => clocksOf(p, found));
        if (n[0] === "M") clocksOf(n[2], found);
        if (n[0] === "S") clocksOf(n[4], found);
        if (n[0] === "E") clocksOf(n[3], found);
        return found;
    }

    self.render = ({ title, shape: seen, view, menus: bar, acts }) => {
        failed = 0;
        const window = document.getElementById("kova-window");
        document.title = title;
        const stamp = JSON.stringify(bar);
        if (window.stamp !== stamp) {
            window.stamp = stamp;
            menus(bar);
        }
        for (const act of acts) {
            if (act === 0) document.body.classList.add("kova-frameless");
            if (act === 1) closed = true;
            if (act === 5) document.documentElement.requestFullscreen?.();
        }
        const style = { color: null, size: 0, bold: false };
        if (seen === shape) update(root, view, style);
        else {
            shape = seen;
            root = build(view, style);
            window.replaceChildren(root);
            update(root, view, style); // sizes are known now that it is in the page
        }
        const ticking = new Set();
        for (const [ms, k] of clocksOf(view, [])) {
            const key = ms + ":" + k;
            ticking.add(key);
            if (!clocks.has(key)) clocks.set(key, setInterval(() => hear(k), ms));
        }
        for (const [key, clock] of clocks) {
            if (!ticking.has(key)) {
                clearInterval(clock);
                clocks.delete(key);
            }
        }
        if (closed) {
            for (const clock of clocks.values()) clearInterval(clock);
            clocks.clear();
            window.replaceChildren();
            queue.length = 0;
            hear(-1);
        }
        return failed;
    };

    self.area = k => {
        if (!web) return 0;
        return [0, 0, innerWidth, innerHeight, 0, 0, screen.width, screen.height][k];
    };
    self.ask = (kind, title, body) => {
        if (kind === 4) return confirm(title + "\n\n" + body);
        if (kind === 3) alert(title + "\n\n" + body);
        return false;
    };
    self.chosen = null;
    self.pick = () =>
        new Promise(done => {
            const input = document.createElement("input");
            input.type = "file";
            input.oncancel = done;
            input.onchange = async () => {
                const file = input.files[0];
                if (file) self.chosen = { name: file.name, data: new Uint8Array(await file.arrayBuffer()) };
                done();
            };
            input.click();
        });
    self.download = (name, data) => {
        const link = document.createElement("a");
        link.href = URL.createObjectURL(new Blob([data]));
        link.download = name;
        link.click();
    };
    self.play = wav =>
        new Promise(done => {
            if (typeof Audio === "undefined") return done();
            const sound = new Audio(URL.createObjectURL(new Blob([wav], { type: "audio/wav" })));
            sound.onended = sound.onerror = done;
            sound.play().catch(done);
        });
    if (web) addEventListener("resize", () => shape !== null && hear(RESIZED));
    return self;
}
