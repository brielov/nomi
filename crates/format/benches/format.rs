//! `cargo xtask bench format`: how long formatting takes on inputs of each
//! shape, at three sizes, each twice the one before. Each must take less
//! than three times as long as the one before, or the run fails:
//! formatting scales linearly. Times are the fastest of several runs.

use std::time::{Duration, Instant};

/// A real module: style-elements' `Element`, from elm-format's tests.
const ELEMENT: &str = include_str!(
    "../tests/format_fixtures/inputs/elm-format/tests/test-files/good/Elm-0.18/mdgriffith/style-elements/Element.elm"
);

/// How much slower twice the input may be.
const SCALING: f64 = 3.0;

struct Shape {
    name: &'static str,
    /// The input of a size.
    input: fn(usize) -> String,
    size: usize,
}

const SHAPES: &[Shape] = &[
    Shape {
        name: "module",
        input: module,
        size: 4,
    },
    Shape {
        name: "nested calls",
        input: |n| nested(n, "f (", ")"),
        size: 2_500,
    },
    Shape {
        name: "nested records",
        input: |n| nested(n, "{ a = ", " }"),
        size: 2_500,
    },
    Shape {
        name: "nested lambdas",
        input: |n| nested(n, "\\a -> ", ""),
        size: 2_500,
    },
    Shape {
        name: "else ifs",
        input: |n| nested(n, "if a then 1 else ", ""),
        size: 2_500,
    },
    Shape {
        name: "operator chain",
        input: |n| nested(n, "1 + ", ""),
        size: 20_000,
    },
];

/// `Element`'s declarations `n` times under its header and imports.
fn module(n: usize) -> String {
    let imports = ELEMENT.rfind("\nimport ").expect("the module imports");
    let body = imports + ELEMENT[imports + 1..].find('\n').expect("a line") + 1;
    let (header, body) = ELEMENT.split_at(body);
    format!("{header}{}", body.repeat(n))
}

/// `x` defined as `open`, then `1`, then `close`, each `n` times.
fn nested(n: usize, open: &str, close: &str) -> String {
    format!(
        "module A exposing (x)\n\n\nx =\n    {}1{}\n",
        open.repeat(n),
        close.repeat(n)
    )
}

fn run(text: &str) {
    assert!(nomi_format::format(text).is_some(), "the input formats");
}

/// The fastest of runs taking at least a second together, and at least
/// five.
fn fastest(text: &str) -> Duration {
    run(text);
    let (mut best, mut total, mut runs) = (Duration::MAX, Duration::ZERO, 0);
    while runs < 5 || total < Duration::from_secs(1) {
        let start = Instant::now();
        run(text);
        let took = start.elapsed();
        best = best.min(took);
        total += took;
        runs += 1;
    }
    best
}

fn main() {
    // Deep trees need the stack the shell gives core work.
    let failed = stacker::grow(nomi_base::STACK_SIZE, || {
        println!(
            "| {:<16} | {:>9} | {:>9} | {:>9} | {:>9} | {:>9} | {:>9} | {:>5} |",
            "shape", "size", "time", "2× size", "time", "4× size", "time", "ratio"
        );
        println!(
            "|{:-<18}|{:->11}|{:->11}|{:->11}|{:->11}|{:->11}|{:->11}|{:->7}|",
            "", "", "", "", "", "", "", ""
        );
        let mut failed = Vec::new();
        for shape in SHAPES {
            let inputs = [1, 2, 4].map(|k| (shape.input)(k * shape.size));
            let times = inputs.each_ref().map(|text| fastest(text));
            let ratio = times
                .windows(2)
                .map(|w| w[1].as_secs_f64() / w[0].as_secs_f64())
                .fold(0.0, f64::max);
            println!(
                "| {:<16} | {:>9} | {:>9} | {:>9} | {:>9} | {:>9} | {:>9} | {ratio:>5.2} |",
                shape.name,
                kib(inputs[0].len()),
                ms(times[0]),
                kib(inputs[1].len()),
                ms(times[1]),
                kib(inputs[2].len()),
                ms(times[2]),
            );
            if ratio >= SCALING {
                failed.push(shape.name);
            }
        }
        failed
    });
    if !failed.is_empty() {
        eprintln!(
            "twice the input took {SCALING} times as long or more: {}",
            failed.join(", ")
        );
        std::process::exit(1);
    }
}

fn kib(bytes: usize) -> String {
    format!("{} KiB", bytes / 1024)
}

fn ms(time: Duration) -> String {
    format!("{:.2} ms", time.as_secs_f64() * 1e3)
}
