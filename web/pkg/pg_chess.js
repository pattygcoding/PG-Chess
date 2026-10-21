/* @ts-self-types="./pg_chess.d.ts" */

/**
 * A wasm-side engine handle.
 */
export class Engine {
    static __wrap(ptr) {
        const obj = Object.create(Engine.prototype);
        obj.__wbg_ptr = ptr;
        EngineFinalization.register(obj, obj.__wbg_ptr, obj);
        return obj;
    }
    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        EngineFinalization.unregister(this);
        return ptr;
    }
    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_engine_free(ptr, 0);
    }
    /**
     * The static evaluation in centipawns, from the side to move's point of view.
     * @returns {number}
     */
    evaluate() {
        const ret = wasm.engine_evaluate(this.__wbg_ptr);
        return ret;
    }
    /**
     * The current position as FEN, which is one of the two ways the page can
     * recreate the board.
     * @returns {string}
     */
    fen() {
        let deferred1_0;
        let deferred1_1;
        try {
            const ret = wasm.engine_fen(this.__wbg_ptr);
            deferred1_0 = ret[0];
            deferred1_1 = ret[1];
            return getStringFromWasm0(ret[0], ret[1]);
        } finally {
            wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * A fresh engine on a FEN position.  Returns an error string when the FEN is
     * not readable, which the page shows next to the input.
     * @param {string} fen
     * @returns {Engine}
     */
    static fromFen(fen) {
        const ptr0 = passStringToWasm0(fen, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.engine_fromFen(ptr0, len0);
        if (ret[2]) {
            throw takeFromExternrefTable0(ret[1]);
        }
        return Engine.__wrap(ret[0]);
    }
    /**
     * True when the side to move is in check.
     * @returns {boolean}
     */
    inCheck() {
        const ret = wasm.engine_inCheck(this.__wbg_ptr);
        return ret !== 0;
    }
    /**
     * True once no further moves can be played.
     * @returns {boolean}
     */
    isGameOver() {
        const ret = wasm.engine_isGameOver(this.__wbg_ptr);
        return ret !== 0;
    }
    /**
     * The last move played, as UCI, or `""` at the start of a game.
     * @returns {string}
     */
    lastMove() {
        let deferred1_0;
        let deferred1_1;
        try {
            const ret = wasm.engine_lastMove(this.__wbg_ptr);
            deferred1_0 = ret[0];
            deferred1_1 = ret[1];
            return getStringFromWasm0(ret[0], ret[1]);
        } finally {
            wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * Every legal move as UCI, space separated.  The page uses this both to
     * validate a click and to light up the squares the selected piece can reach.
     * @returns {string}
     */
    legalMoves() {
        let deferred1_0;
        let deferred1_1;
        try {
            const ret = wasm.engine_legalMoves(this.__wbg_ptr);
            deferred1_0 = ret[0];
            deferred1_1 = ret[1];
            return getStringFromWasm0(ret[0], ret[1]);
        } finally {
            wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * The side that has been mated, or `""` while the game is still running.
     *
     * `outcome() == "checkmate"` says somebody was mated; this says who, and the
     * answer is always the side to move.
     * @returns {string}
     */
    matedSide() {
        let deferred1_0;
        let deferred1_1;
        try {
            const ret = wasm.engine_matedSide(this.__wbg_ptr);
            deferred1_0 = ret[0];
            deferred1_1 = ret[1];
            return getStringFromWasm0(ret[0], ret[1]);
        } finally {
            wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * Starts a fresh game from a FEN.
     * @param {string} fen
     */
    newGameFromFen(fen) {
        const ptr0 = passStringToWasm0(fen, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.engine_newGameFromFen(this.__wbg_ptr, ptr0, len0);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * Starts a fresh game from the standard position.
     */
    newGame() {
        wasm.engine_newGame(this.__wbg_ptr);
    }
    /**
     * A fresh engine on the standard starting position.
     */
    constructor() {
        const ret = wasm.engine_new();
        this.__wbg_ptr = ret;
        EngineFinalization.register(this, this.__wbg_ptr, this);
        return this;
    }
    /**
     * `playing`, `checkmate`, `stalemate`, `fifty-move`, `repetition` or
     * `insufficient-material`.
     * @returns {string}
     */
    outcome() {
        let deferred1_0;
        let deferred1_1;
        try {
            const ret = wasm.engine_outcome(this.__wbg_ptr);
            deferred1_0 = ret[0];
            deferred1_1 = ret[1];
            return getStringFromWasm0(ret[0], ret[1]);
        } finally {
            wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * The per-root-move counts, as `"e2e4:20 g1f3:20"`.
     * @param {number} depth
     * @returns {string}
     */
    perftDivide(depth) {
        let deferred1_0;
        let deferred1_1;
        try {
            const ret = wasm.engine_perftDivide(this.__wbg_ptr, depth);
            deferred1_0 = ret[0];
            deferred1_1 = ret[1];
            return getStringFromWasm0(ret[0], ret[1]);
        } finally {
            wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * Counts leaf nodes to `depth`, for the harness's own perft checks.
     * @param {number} depth
     * @returns {number}
     */
    perft(depth) {
        const ret = wasm.engine_perft(this.__wbg_ptr, depth);
        return ret;
    }
    /**
     * The board as 64 characters, `a8` first and `h1` last: `P` is a white pawn,
     * `n` a black knight, `.` an empty square.
     *
     * The page draws this rather than parsing the FEN again, because a flat
     * string needs no parsing at all.
     * @returns {string}
     */
    piecePlacement() {
        let deferred1_0;
        let deferred1_1;
        try {
            const ret = wasm.engine_piecePlacement(this.__wbg_ptr);
            deferred1_0 = ret[0];
            deferred1_1 = ret[1];
            return getStringFromWasm0(ret[0], ret[1]);
        } finally {
            wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * Plays a legal move given in UCI and returns the SAN of what was played.
     *
     * Returns an error string when the move is not legal here, which is what lets
     * the page treat a rejection as "that click was not a move".
     * @param {string} uci
     * @returns {string}
     */
    play(uci) {
        let deferred3_0;
        let deferred3_1;
        try {
            const ptr0 = passStringToWasm0(uci, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            const len0 = WASM_VECTOR_LEN;
            const ret = wasm.engine_play(this.__wbg_ptr, ptr0, len0);
            var ptr2 = ret[0];
            var len2 = ret[1];
            if (ret[3]) {
                ptr2 = 0; len2 = 0;
                throw takeFromExternrefTable0(ret[2]);
            }
            deferred3_0 = ptr2;
            deferred3_1 = len2;
            return getStringFromWasm0(ptr2, len2);
        } finally {
            wasm.__wbindgen_free(deferred3_0, deferred3_1, 1);
        }
    }
    /**
     * Plies played since this engine was created or restarted.
     * @returns {number}
     */
    plyCount() {
        const ret = wasm.engine_plyCount(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * Forgets the search heuristics, for a new game.
     */
    resetSearch() {
        wasm.engine_resetSearch(this.__wbg_ptr);
    }
    /**
     * The SAN of a move without playing it.
     * @param {string} uci
     * @returns {string}
     */
    san(uci) {
        let deferred2_0;
        let deferred2_1;
        try {
            const ptr0 = passStringToWasm0(uci, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            const len0 = WASM_VECTOR_LEN;
            const ret = wasm.engine_san(this.__wbg_ptr, ptr0, len0);
            deferred2_0 = ret[0];
            deferred2_1 = ret[1];
            return getStringFromWasm0(ret[0], ret[1]);
        } finally {
            wasm.__wbindgen_free(deferred2_0, deferred2_1, 1);
        }
    }
    /**
     * Searches and returns the result.  A `max_depth` or `time_ms` of zero means
     * "whatever this level's setting is".
     * @param {number} max_depth
     * @param {number} time_ms
     * @param {number} level
     * @returns {SearchInfo}
     */
    search(max_depth, time_ms, level) {
        const ret = wasm.engine_search(this.__wbg_ptr, max_depth, time_ms, level);
        return SearchInfo.__wrap(ret);
    }
    /**
     * Resizes the transposition table, in megabytes.
     * @param {number} megabytes
     */
    setTtSize(megabytes) {
        wasm.engine_setTtSize(this.__wbg_ptr, megabytes);
    }
    /**
     * `"white"` or `"black"`.
     * @returns {string}
     */
    sideToMove() {
        let deferred1_0;
        let deferred1_1;
        try {
            const ret = wasm.engine_sideToMove(this.__wbg_ptr);
            deferred1_0 = ret[0];
            deferred1_1 = ret[1];
            return getStringFromWasm0(ret[0], ret[1]);
        } finally {
            wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * The table's size in bytes.
     * @returns {number}
     */
    ttSize() {
        const ret = wasm.engine_ttSize(this.__wbg_ptr);
        return ret;
    }
    /**
     * Takes one ply back.  The page calls it twice to take back a full move.
     * @returns {boolean}
     */
    undo() {
        const ret = wasm.engine_undo(this.__wbg_ptr);
        return ret !== 0;
    }
}
if (Symbol.dispose) Engine.prototype[Symbol.dispose] = Engine.prototype.free;

/**
 * What a search returns to JavaScript.
 */
export class SearchInfo {
    static __wrap(ptr) {
        const obj = Object.create(SearchInfo.prototype);
        obj.__wbg_ptr = ptr;
        SearchInfoFinalization.register(obj, obj.__wbg_ptr, obj);
        return obj;
    }
    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        SearchInfoFinalization.unregister(this);
        return ptr;
    }
    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_searchinfo_free(ptr, 0);
    }
    /**
     * The deepest completed iteration.
     * @returns {number}
     */
    depth() {
        const ret = wasm.searchinfo_depth(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * Moves until mate when the score is a mate, otherwise 0.  Negative means the
     * side to move is the one being mated.
     * @returns {number}
     */
    get mateIn() {
        const ret = wasm.searchinfo_mateIn(this.__wbg_ptr);
        return ret;
    }
    /**
     * Nodes per second.
     * @returns {number}
     */
    get nodesPerSecond() {
        const ret = wasm.searchinfo_nodesPerSecond(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * Nodes visited.
     * @returns {number}
     */
    nodes() {
        const ret = wasm.searchinfo_nodes(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * The principal variation in UCI, space separated.
     * @returns {string}
     */
    pv() {
        let deferred1_0;
        let deferred1_1;
        try {
            const ret = wasm.searchinfo_pv(this.__wbg_ptr);
            deferred1_0 = ret[0];
            deferred1_1 = ret[1];
            return getStringFromWasm0(ret[0], ret[1]);
        } finally {
            wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * The best move in SAN.
     * @returns {string}
     */
    san() {
        let deferred1_0;
        let deferred1_1;
        try {
            const ret = wasm.searchinfo_san(this.__wbg_ptr);
            deferred1_0 = ret[0];
            deferred1_1 = ret[1];
            return getStringFromWasm0(ret[0], ret[1]);
        } finally {
            wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
        }
    }
    /**
     * Centipawns, from the side to move's point of view.
     * @returns {number}
     */
    score() {
        const ret = wasm.searchinfo_score(this.__wbg_ptr);
        return ret;
    }
    /**
     * Milliseconds spent.
     * @returns {number}
     */
    get timeMs() {
        const ret = wasm.searchinfo_timeMs(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * The best move in UCI.
     * @returns {string}
     */
    uci() {
        let deferred1_0;
        let deferred1_1;
        try {
            const ret = wasm.searchinfo_uci(this.__wbg_ptr);
            deferred1_0 = ret[0];
            deferred1_1 = ret[1];
            return getStringFromWasm0(ret[0], ret[1]);
        } finally {
            wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
        }
    }
}
if (Symbol.dispose) SearchInfo.prototype[Symbol.dispose] = SearchInfo.prototype.free;
function __wbg_get_imports() {
    const import0 = {
        __proto__: null,
        __wbg___wbindgen_throw_41e9ee4f547fc59a: function(arg0, arg1) {
            throw new Error(getStringFromWasm0(arg0, arg1));
        },
        __wbg_now_aa4ccb83129e9e55: function() {
            const ret = Date.now();
            return ret;
        },
        __wbindgen_generic_0000000000000001: function(arg0, arg1) {
            // Cast intrinsic for `Ref(String) -> Externref`.
            const ret = getStringFromWasm0(arg0, arg1);
            return ret;
        },
        __wbindgen_init_externref_table: function() {
            const table = wasm.__wbindgen_externrefs;
            const offset = table.grow(4);
            table.set(0, undefined);
            table.set(offset + 0, undefined);
            table.set(offset + 1, null);
            table.set(offset + 2, true);
            table.set(offset + 3, false);
        },
    };
    return {
        __proto__: null,
        "./pg_chess_bg.js": import0,
    };
}

const EngineFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_engine_free(ptr, 1));
const SearchInfoFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_searchinfo_free(ptr, 1));

function getStringFromWasm0(ptr, len) {
    return decodeText(ptr >>> 0, len);
}

let cachedUint8ArrayMemory0 = null;
function getUint8ArrayMemory0() {
    if (cachedUint8ArrayMemory0 === null || cachedUint8ArrayMemory0.byteLength === 0) {
        cachedUint8ArrayMemory0 = new Uint8Array(wasm.memory.buffer);
    }
    return cachedUint8ArrayMemory0;
}

function passStringToWasm0(arg, malloc, realloc) {
    if (realloc === undefined) {
        const buf = cachedTextEncoder.encode(arg);
        const ptr = malloc(buf.length, 1) >>> 0;
        getUint8ArrayMemory0().subarray(ptr, ptr + buf.length).set(buf);
        WASM_VECTOR_LEN = buf.length;
        return ptr;
    }

    let len = arg.length;
    let ptr = malloc(len, 1) >>> 0;

    const mem = getUint8ArrayMemory0();

    let offset = 0;

    for (; offset < len; offset++) {
        const code = arg.charCodeAt(offset);
        if (code > 0x7F) break;
        mem[ptr + offset] = code;
    }
    if (offset !== len) {
        if (offset !== 0) {
            arg = arg.slice(offset);
        }
        ptr = realloc(ptr, len, len = offset + arg.length * 3, 1) >>> 0;
        const view = getUint8ArrayMemory0().subarray(ptr + offset, ptr + len);
        const ret = cachedTextEncoder.encodeInto(arg, view);

        offset += ret.written;
        ptr = realloc(ptr, len, offset, 1) >>> 0;
    }

    WASM_VECTOR_LEN = offset;
    return ptr;
}

function takeFromExternrefTable0(idx) {
    const value = wasm.__wbindgen_externrefs.get(idx);
    wasm.__externref_table_dealloc(idx);
    return value;
}

let cachedTextDecoder = new TextDecoder('utf-8', { ignoreBOM: true, fatal: true });
cachedTextDecoder.decode();
const MAX_SAFARI_DECODE_BYTES = 2146435072;
let numBytesDecoded = 0;
function decodeText(ptr, len) {
    numBytesDecoded += len;
    if (numBytesDecoded >= MAX_SAFARI_DECODE_BYTES) {
        cachedTextDecoder = new TextDecoder('utf-8', { ignoreBOM: true, fatal: true });
        cachedTextDecoder.decode();
        numBytesDecoded = len;
    }
    return cachedTextDecoder.decode(getUint8ArrayMemory0().subarray(ptr, ptr + len));
}

const cachedTextEncoder = new TextEncoder();

if (!('encodeInto' in cachedTextEncoder)) {
    cachedTextEncoder.encodeInto = function (arg, view) {
        const buf = cachedTextEncoder.encode(arg);
        view.set(buf);
        return {
            read: arg.length,
            written: buf.length
        };
    };
}

let WASM_VECTOR_LEN = 0;

let wasmModule, wasmInstance, wasm;
function __wbg_finalize_init(instance, module) {
    wasmInstance = instance;
    wasm = instance.exports;
    wasmModule = module;
    cachedUint8ArrayMemory0 = null;
    wasm.__wbindgen_start();
    return wasm;
}

async function __wbg_load(module, imports) {
    if (typeof Response === 'function' && module instanceof Response) {
        if (!module.ok) {
            throw new Error(`failed to fetch Wasm: ${module.status} ${module.statusText} fetching '${module.url}'`);
        }

        if (typeof WebAssembly.instantiateStreaming === 'function') {
            try {
                return await WebAssembly.instantiateStreaming(module, imports);
            } catch (e) {
                const validResponse = expectedResponseType(module.type);

                if (validResponse && module.headers.get('Content-Type') !== 'application/wasm') {
                    console.warn("`WebAssembly.instantiateStreaming` failed because your server does not serve Wasm with `application/wasm` MIME type. Falling back to `WebAssembly.instantiate` which is slower. Original error:\n", e);

                } else { throw e; }
            }
        }

        const bytes = await module.arrayBuffer();
        return await WebAssembly.instantiate(bytes, imports);
    } else {
        const instance = await WebAssembly.instantiate(module, imports);

        if (instance instanceof WebAssembly.Instance) {
            return { instance, module };
        } else {
            return instance;
        }
    }

    function expectedResponseType(type) {
        switch (type) {
            case 'basic': case 'cors': case 'default': return true;
        }
        return false;
    }
}

function initSync(module) {
    if (wasm !== undefined) return wasm;


    if (module !== undefined) {
        if (Object.getPrototypeOf(module) === Object.prototype) {
            ({module} = module)
        } else {
            console.warn('using deprecated parameters for `initSync()`; pass a single object instead')
        }
    }

    const imports = __wbg_get_imports();
    if (!(module instanceof WebAssembly.Module)) {
        module = new WebAssembly.Module(module);
    }
    const instance = new WebAssembly.Instance(module, imports);
    return __wbg_finalize_init(instance, module);
}

async function __wbg_init(module_or_path) {
    if (wasm !== undefined) return wasm;


    if (module_or_path !== undefined) {
        if (Object.getPrototypeOf(module_or_path) === Object.prototype) {
            ({module_or_path} = module_or_path)
        } else {
            console.warn('using deprecated parameters for the initialization function; pass a single object instead')
        }
    }

    if (module_or_path === undefined) {
        module_or_path = new URL('pg_chess_bg.wasm', import.meta.url);
    }
    const imports = __wbg_get_imports();

    if (typeof module_or_path === 'string' || (typeof Request === 'function' && module_or_path instanceof Request) || (typeof URL === 'function' && module_or_path instanceof URL)) {
        module_or_path = fetch(module_or_path);
    }

    const { instance, module } = await __wbg_load(await module_or_path, imports);

    return __wbg_finalize_init(instance, module);
}

export { initSync, __wbg_init as default };
