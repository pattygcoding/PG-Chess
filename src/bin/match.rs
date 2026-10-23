//! Self-play matches: the engine against itself, one technique set per side.
//!
//!     cargo run --release --bin match -- --games 200 --ms 60
//!     cargo run --release --bin match -- --games 40 --ms 0 --depth 6 --verbose
//!
//! The two sides are the same engine.  The only difference is `SearchFeatures`, so
//! the result is about those features and nothing else: same move generator, same
//! evaluation tables, same transposition table size, and the same opening lines
//! played by both sides in both colours.  A match is the only honest way to say that
//! the work that went into the search and the evaluation made the engine stronger
//! rather than merely busier.
//!
//! The report is a win/draw/loss count for `now` (everything on) against `old`
//! (everything off), the score as a share of the games, and an Elo estimate with a
//! 95% interval.  The interval is the point: a hundred games cannot resolve a ten-Elo
//! difference, and the report says so instead of pretending otherwise.

use pg_chess::board::{Board, Outcome, WHITE};
use pg_chess::evaluate;
use pg_chess::moves;
use pg_chess::search::{Level, SearchFeatures, SearchLimits, Searcher};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::thread;

/// A day, so the depth cap is the only limit in a fixed-depth match.
const NO_TIME_LIMIT: u64 = 86_400_000;

/// Games that run this long without a result are counted as draws.  A hundred and
/// twenty plies is well past the point where anything is still being decided.
const MAX_PLIES: usize = 120;

/// Short openings, in UCI, played before the engines take over.  Four plies each,
/// and every line is played with each side on the white pieces, so a win for one
/// feature set cannot come from the line itself.
const OPENINGS: [[&str; 4]; 8] = [
    ["e2e4", "e7e5", "g1f3", "b8c6"],
    ["e2e4", "c7c5", "g1f3", "d7d6"],
    ["d2d4", "d7d5", "c2c4", "e7e6"],
    ["d2d4", "g8f6", "c2c4", "e7e6"],
    ["e2e4", "e7e5", "f1c4", "g8f6"],
    ["g1f3", "d7d5", "d2d4", "g8f6"],
    ["c2c4", "e7e5", "b1c3", "g8f6"],
    ["e2e4", "e7e5", "g1f3", "g8f6"],
];

/// How one game ended, from the point of view of the `now` side.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Result {
    Win,
    Draw,
    Loss,
}

fn name(result: Result) -> &'static str {
    match result {
        Result::Win => "win",
        Result::Draw => "draw",
        Result::Loss => "loss",
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Opponent {
    /// The engine as the first release shipped it: no technique flags and the
    /// evaluation written from white's side.  This is the headline comparison.
    Shipped,
    /// The same search with the technique flags off but the evaluation read from the
    /// side to move: isolates what the search and evaluation *additions* are worth.
    Features,
    /// Every technique on, but the evaluation read from white's side: isolates the
    /// fix to the score's orientation.
    Orientation,
    /// Every technique on: the control, and the base for `--without`, which turns
    /// this mode into "the same engine minus one technique".
    Everything,
    /// The engine as the previous release shipped: this round's techniques all off,
    /// including the mobility term that is on in `all()`.  This is the comparison that
    /// says what this round of work was worth.
    Previous,
}

impl Opponent {
    fn features(self) -> SearchFeatures {
        match self {
            Opponent::Shipped => SearchFeatures {
                eval: evaluate::EvalFeatures::as_shipped(),
                ..SearchFeatures::baseline()
            },
            Opponent::Features => SearchFeatures::baseline(),
            Opponent::Orientation => SearchFeatures {
                eval: evaluate::EvalFeatures {
                    white_positive_score: true,
                    ..evaluate::EvalFeatures::all()
                },
                ..SearchFeatures::all()
            },
            Opponent::Everything => SearchFeatures::all(),
            Opponent::Previous => SearchFeatures {
                lmr_refinements: false,
                eval: evaluate::EvalFeatures {
                    mobility: false,
                    ..evaluate::EvalFeatures::all()
                },
                ..SearchFeatures::all()
            },
        }
    }

    fn name(self) -> &'static str {
        match self {
            Opponent::Shipped => "the engine as shipped",
            Opponent::Features => "the same search without the new techniques",
            Opponent::Orientation => "the same engine with the score read from white",
            Opponent::Everything => "the same engine, every technique on",
            Opponent::Previous => "the previous release",
        }
    }
}

#[derive(Clone)]
struct Options {
    games: usize,
    /// Milliseconds per move.  Zero means "search to the depth cap, however long it
    /// takes", which makes a match reproducible move for move.
    time_ms: u64,
    depth: i32,
    threads: usize,
    verbose: bool,
    opponent: Opponent,
    /// Flags to clear on the opponent's side *in addition* to whatever its mode
    /// implies, which is how one technique is measured on its own: the opponent is
    /// the whole engine minus that flag, so the match says what it is worth.
    without: Vec<String>,
}

impl Options {
    /// The opponent's feature set: its mode, minus anything named with `--without`.
    fn opponent_features(&self) -> SearchFeatures {
        let mut features = self.opponent.features();
        for name in &self.without {
            match name.as_str() {
                "see_pruning" => features.see_pruning = false,
                "see_ordering" => features.see_ordering = false,
                "aspiration" => features.aspiration = false,
                "reverse_futility" => features.reverse_futility = false,
                "futility" => features.futility = false,
                "late_move_pruning" => features.late_move_pruning = false,
                "internal_iterative_deepening" => features.internal_iterative_deepening = false,
                "log_reductions" => features.log_reductions = false,
                "counter_moves" => features.counter_moves = false,
                "lmr_refinements" => features.lmr_refinements = false,
                "soft_time_limit" => features.soft_time_limit = false,
                "mobility" => features.eval.mobility = false,
                "threats" => features.eval.threats = false,
                "king_safety" => features.eval.king_safety = false,
                "passed_pawn_king_proximity" => features.eval.passed_pawn_king_proximity = false,
                other => panic!("--without does not know {}", other),
            }
        }
        features
    }
}

impl Options {
    fn from_arguments() -> Options {
        let mut options = Options {
            games: 100,
            time_ms: 60,
            depth: 8,
            threads: 0,
            verbose: false,
            opponent: Opponent::Shipped,
            without: Vec::new(),
        };
        let arguments: Vec<String> = std::env::args().skip(1).collect();
        let mut index = 0;
        while index < arguments.len() {
            let number = |at: usize| -> u64 {
                arguments
                    .get(at + 1)
                    .and_then(|text| text.parse::<u64>().ok())
                    .unwrap_or_else(|| panic!("{} needs a number", arguments[at]))
            };
            match arguments[index].as_str() {
                "--games" => {
                    options.games = number(index) as usize;
                    index += 1;
                }
                "--ms" => {
                    options.time_ms = number(index);
                    index += 1;
                }
                "--depth" => {
                    options.depth = number(index) as i32;
                    index += 1;
                }
                "--threads" => {
                    options.threads = number(index) as usize;
                    index += 1;
                }
                "--verbose" => options.verbose = true,
                "--without" => {
                    let name = arguments
                        .get(index + 1)
                        .unwrap_or_else(|| panic!("--without needs a flag name"))
                        .clone();
                    options.without.push(name);
                    index += 1;
                }
                "--old" => {
                    options.opponent = match arguments.get(index + 1).map(|text| text.as_str()) {
                        Some("shipped") => Opponent::Shipped,
                        Some("features") => Opponent::Features,
                        Some("orientation") => Opponent::Orientation,
                        Some("full") => Opponent::Everything,
                        Some("previous") => Opponent::Previous,
                        other => panic!(
                            "--old takes shipped, features, orientation, full or previous, not {:?}",
                            other
                        ),
                    };
                    index += 1;
                }
                other => panic!("unknown argument {}", other),
            }
            index += 1;
        }
        if options.threads == 0 {
            options.threads = thread::available_parallelism()
                .map(|count| count.get())
                .unwrap_or(4);
        }
        options
    }
}

fn main() {
    let options = Options::from_arguments();
    println!("PG-Chess self-play: now (everything on) vs {}", options.opponent.name());
    println!(
        "  {} games, {} per move, {} ply cap, {} threads, {} openings\n",
        options.games,
        if options.time_ms == 0 {
            "no clock".to_string()
        } else {
            format!("{} ms", options.time_ms)
        },
        options.depth,
        options.threads,
        OPENINGS.len()
    );

    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<(usize, Result, usize)>> = Mutex::new(Vec::new());
    let started = std::time::Instant::now();

    thread::scope(|scope| {
        for _ in 0..options.threads {
            scope.spawn(|| {
                // One searcher per feature set per thread: the table, the history
                // counters and the killers are all per searcher, and sharing them
                // across threads would need a lock inside the hot loop.
                let mut now = Searcher::new(8);
                now.set_features(SearchFeatures::all());
                let mut old = Searcher::new(8);
                old.set_features(options.opponent_features());

                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    if index >= options.games {
                        break;
                    }
                    let (result, plies, line) = play(&mut now, &mut old, index, &options);
                    let done = {
                        let mut results = results.lock().unwrap();
                        results.push((index, result, plies));
                        results.len()
                    };
                    if options.verbose {
                        println!(
                            "  game {:>4} {:<4} in {:>3} plies  {}",
                            index,
                            name(result),
                            plies,
                            line
                        );
                    } else if done % 20 == 0 {
                        println!("  ... {} of {} games", done, options.games);
                    }
                }
            });
        }
    });

    let mut results = results.into_inner().unwrap();
    results.sort_by_key(|(index, _, _)| *index);
    report(&results, started.elapsed().as_secs_f64());
}

/// One game, from an opening line to a result.
fn play(
    now: &mut Searcher,
    old: &mut Searcher,
    index: usize,
    options: &Options,
) -> (Result, usize, String) {
    // Every opening is played twice: once with the new features on the white pieces
    // and once with them on the black pieces.  Anything else would measure the
    // advantage of the first move.
    let opening = &OPENINGS[(index / 2) % OPENINGS.len()];
    let now_is_white = index % 2 == 0;

    let mut board = Board::startpos();
    let mut line = String::new();
    for text in opening {
        let mv = moves::from_uci(text).expect("opening move");
        assert!(board.make_move(mv), "opening move {} is illegal", text);
        line.push_str(&format!("{} ", text));
    }

    let mut plies = opening.len();
    let mut outcome = board.outcome();
    while outcome == Outcome::Playing && plies < MAX_PLIES {
        let now_to_move = (board.side_to_move() == WHITE) == now_is_white;
        let searcher = if now_to_move { &mut *now } else { &mut *old };
        let limits = SearchLimits {
            max_depth: options.depth,
            time_ms: if options.time_ms == 0 {
                NO_TIME_LIMIT
            } else {
                options.time_ms
            },
            level: Level::Strong,
        };
        // The table and the history counters describe *this* game: carrying them
        // across games would have both sides play the previous game's positions.
        searcher.reset_heuristics();
        let result = searcher.search(&mut board, limits);
        assert!(result.best != 0, "the search returned no move");
        let played = result.best;
        assert!(
            board.make_move(played),
            "the search returned an illegal move: {}",
            moves::to_uci(played)
        );
        line.push_str(&format!("{}({}) ", moves::to_uci(played), result.depth));
        plies += 1;
        outcome = board.outcome();
    }

    let result = match outcome {
        // Nothing was decided inside the ply limit, which counts as a draw.
        Outcome::Playing => Result::Draw,
        Outcome::Checkmate => {
            // The side to move is the one that has been mated.
            let mated_is_white = board.side_to_move() == WHITE;
            if mated_is_white == now_is_white {
                Result::Loss
            } else {
                Result::Win
            }
        }
        _ => Result::Draw,
    };
    (result, plies, line)
}

/// The numbers: score, Elo estimate and the 95% interval the match can support.
fn report(results: &[(usize, Result, usize)], seconds: f64) {
    let games = results.len() as f64;
    let wins = results.iter().filter(|(_, r, _)| *r == Result::Win).count();
    let draws = results.iter().filter(|(_, r, _)| *r == Result::Draw).count();
    let losses = results.iter().filter(|(_, r, _)| *r == Result::Loss).count();
    let scores: Vec<f64> = results
        .iter()
        .map(|(_, result, _)| match result {
            Result::Win => 1.0,
            Result::Draw => 0.5,
            Result::Loss => 0.0,
        })
        .collect();
    let mean = scores.iter().sum::<f64>() / games;
    let variance = if games > 1.0 {
        scores.iter().map(|s| (s - mean).powi(2)).sum::<f64>() / (games - 1.0)
    } else {
        0.0
    };
    let stderr = (variance / games).sqrt();
    let average_plies = results.iter().map(|(_, _, plies)| *plies).sum::<usize>() as f64 / games;

    println!(
        "\nnow {} wins, {} draws, {} losses: {:.0}% of the points",
        wins,
        draws,
        losses,
        mean * 100.0
    );
    println!(
        "  {:.0} seconds of wall clock, {:.0} plies per game on average",
        seconds, average_plies
    );

    // Elo = 400 log10(score / (1 - score)), with the usual delta-method interval.
    // The conversion stretches near a shutout, which is why a lopsided match gets a
    // wide bar rather than a precise number: the interval is the honest part.
    if mean <= 0.0 || mean >= 1.0 {
        println!(
            "  Elo: outside what a finite estimate can describe ({:.0}% of the points)",
            mean * 100.0
        );
        return;
    }
    let elo = 400.0 * (mean / (1.0 - mean)).log10();
    let elo_error = (400.0 / std::f64::consts::LN_10) * stderr / (mean * (1.0 - mean));
    println!(
        "  Elo: {:+.0}  (95% interval {:+.0} to {:+.0}, a standard error of {:.0})",
        elo,
        elo - 1.96 * elo_error,
        elo + 1.96 * elo_error,
        elo_error
    );
}

