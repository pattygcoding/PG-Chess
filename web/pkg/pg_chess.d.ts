/* tslint:disable */
/* eslint-disable */

/**
 * A wasm-side engine handle.
 */
export class Engine {
    free(): void;
    [Symbol.dispose](): void;
    /**
     * The static evaluation in centipawns, from the side to move's point of view.
     */
    evaluate(): number;
    /**
     * The current position as FEN, which is one of the two ways the page can
     * recreate the board.
     */
    fen(): string;
    /**
     * A fresh engine on a FEN position.  Returns an error string when the FEN is
     * not readable, which the page shows next to the input.
     */
    static fromFen(fen: string): Engine;
    /**
     * True when the side to move is in check.
     */
    inCheck(): boolean;
    /**
     * True once no further moves can be played.
     */
    isGameOver(): boolean;
    /**
     * The last move played, as UCI, or `""` at the start of a game.
     */
    lastMove(): string;
    /**
     * Every legal move as UCI, space separated.  The page uses this both to
     * validate a click and to light up the squares the selected piece can reach.
     */
    legalMoves(): string;
    /**
     * The side that has been mated, or `""` while the game is still running.
     *
     * `outcome() == "checkmate"` says somebody was mated; this says who, and the
     * answer is always the side to move.
     */
    matedSide(): string;
    /**
     * Starts a fresh game from a FEN.
     */
    newGameFromFen(fen: string): void;
    /**
     * Starts a fresh game from the standard position.
     */
    newGame(): void;
    /**
     * A fresh engine on the standard starting position.
     */
    constructor();
    /**
     * `playing`, `checkmate`, `stalemate`, `fifty-move`, `repetition` or
     * `insufficient-material`.
     */
    outcome(): string;
    /**
     * The per-root-move counts, as `"e2e4:20 g1f3:20"`.
     */
    perftDivide(depth: number): string;
    /**
     * Counts leaf nodes to `depth`, for the harness's own perft checks.
     */
    perft(depth: number): number;
    /**
     * The board as 64 characters, `a8` first and `h1` last: `P` is a white pawn,
     * `n` a black knight, `.` an empty square.
     *
     * The page draws this rather than parsing the FEN again, because a flat
     * string needs no parsing at all.
     */
    piecePlacement(): string;
    /**
     * Plays a legal move given in UCI and returns the SAN of what was played.
     *
     * Returns an error string when the move is not legal here, which is what lets
     * the page treat a rejection as "that click was not a move".
     */
    play(uci: string): string;
    /**
     * Plies played since this engine was created or restarted.
     */
    plyCount(): number;
    /**
     * Forgets the search heuristics, for a new game.
     */
    resetSearch(): void;
    /**
     * The SAN of a move without playing it.
     */
    san(uci: string): string;
    /**
     * Searches and returns the result.  A `max_depth` or `time_ms` of zero means
     * "whatever this level's setting is".
     */
    search(max_depth: number, time_ms: number, level: number): SearchInfo;
    /**
     * Resizes the transposition table, in megabytes.
     */
    setTtSize(megabytes: number): void;
    /**
     * `"white"` or `"black"`.
     */
    sideToMove(): string;
    /**
     * The table's size in bytes.
     */
    ttSize(): number;
    /**
     * Takes one ply back.  The page calls it twice to take back a full move.
     */
    undo(): boolean;
}

/**
 * What a search returns to JavaScript.
 */
export class SearchInfo {
    private constructor();
    free(): void;
    [Symbol.dispose](): void;
    /**
     * The deepest completed iteration.
     */
    depth(): number;
    /**
     * Nodes visited.
     */
    nodes(): number;
    /**
     * The principal variation in UCI, space separated.
     */
    pv(): string;
    /**
     * The best move in SAN.
     */
    san(): string;
    /**
     * Centipawns, from the side to move's point of view.
     */
    score(): number;
    /**
     * The best move in UCI.
     */
    uci(): string;
    /**
     * Moves until mate when the score is a mate, otherwise 0.  Negative means the
     * side to move is the one being mated.
     */
    readonly mateIn: number;
    /**
     * Nodes per second.
     */
    readonly nodesPerSecond: number;
    /**
     * Milliseconds spent.
     */
    readonly timeMs: number;
}

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly __wbg_engine_free: (a: number, b: number) => void;
    readonly __wbg_searchinfo_free: (a: number, b: number) => void;
    readonly engine_evaluate: (a: number) => number;
    readonly engine_fen: (a: number) => [number, number];
    readonly engine_fromFen: (a: number, b: number) => [number, number, number];
    readonly engine_inCheck: (a: number) => number;
    readonly engine_isGameOver: (a: number) => number;
    readonly engine_lastMove: (a: number) => [number, number];
    readonly engine_legalMoves: (a: number) => [number, number];
    readonly engine_matedSide: (a: number) => [number, number];
    readonly engine_new: () => number;
    readonly engine_newGame: (a: number) => void;
    readonly engine_newGameFromFen: (a: number, b: number, c: number) => [number, number];
    readonly engine_outcome: (a: number) => [number, number];
    readonly engine_perft: (a: number, b: number) => number;
    readonly engine_perftDivide: (a: number, b: number) => [number, number];
    readonly engine_piecePlacement: (a: number) => [number, number];
    readonly engine_play: (a: number, b: number, c: number) => [number, number, number, number];
    readonly engine_plyCount: (a: number) => number;
    readonly engine_resetSearch: (a: number) => void;
    readonly engine_san: (a: number, b: number, c: number) => [number, number];
    readonly engine_search: (a: number, b: number, c: number, d: number) => number;
    readonly engine_setTtSize: (a: number, b: number) => void;
    readonly engine_sideToMove: (a: number) => [number, number];
    readonly engine_ttSize: (a: number) => number;
    readonly engine_undo: (a: number) => number;
    readonly searchinfo_depth: (a: number) => number;
    readonly searchinfo_mateIn: (a: number) => number;
    readonly searchinfo_nodes: (a: number) => number;
    readonly searchinfo_nodesPerSecond: (a: number) => number;
    readonly searchinfo_pv: (a: number) => [number, number];
    readonly searchinfo_san: (a: number) => [number, number];
    readonly searchinfo_score: (a: number) => number;
    readonly searchinfo_timeMs: (a: number) => number;
    readonly searchinfo_uci: (a: number) => [number, number];
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __wbindgen_free: (a: number, b: number, c: number) => void;
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
    readonly __externref_table_dealloc: (a: number) => void;
    readonly __wbindgen_start: () => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;

/**
 * Instantiates the given `module`, which can either be bytes or
 * a precompiled `WebAssembly.Module`.
 *
 * @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
 *
 * @returns {InitOutput}
 */
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
 * If `module_or_path` is {RequestInfo} or {URL}, makes a request and
 * for everything else, calls `WebAssembly.instantiate` directly.
 *
 * @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
 *
 * @returns {Promise<InitOutput>}
 */
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
