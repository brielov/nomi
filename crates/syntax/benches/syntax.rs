//! `cargo xtask bench syntax`: how long parsing and validation take on
//! inputs of each shape, at two sizes. The second is twice the first, and
//! must take less than three times as long, or the run fails: parsing
//! scales linearly. Times are the fastest of several runs.

use std::time::{Duration, Instant};

use nomi_base::Compat;
use nomi_syntax::{Source, Tool, parse_module, validate};

const DECLARATIONS: &str = include_str!("../tests/syntax_mutations/seeds/Declarations.elm");

/// How much slower twice the input may be.
const SCALING: f64 = 3.0;

struct Shape {
    name: &'static str,
    /// The input of a size.
    input: fn(usize) -> String,
    size: usize,
    run: fn(&str),
}

const SHAPES: &[Shape] = &[
    Shape {
        name: "declarations",
        input: declarations,
        size: 64,
        run: parse,
    },
    Shape {
        name: "declarations, validated",
        input: declarations,
        size: 64,
        run: parse_and_validate,
    },
    Shape {
        name: "nested parentheses",
        input: |n| format!("x = {}1{}\n", "(".repeat(n), ")".repeat(n)),
        size: 10_000,
        run: parse,
    },
    Shape {
        name: "operator chain",
        input: |n| format!("x =\n    1{}\n", " + f 1".repeat(n)),
        size: 50_000,
        run: parse,
    },
    Shape {
        name: "list",
        input: |n| format!("x =\n    [ 1{}\n    ]\n", "\n    , \"s\"".repeat(n)),
        size: 50_000,
        run: parse,
    },
    Shape {
        name: "shader",
        input: |n| {
            let body = "\n  x = vec4(a.xy, 1.0) * m[2] + f(b, c);".repeat(n);
            format!("x =\n    [glsl|\nvoid main () {{{body}\n}}\n|]\n")
        },
        size: 10_000,
        run: parse_and_validate,
    },
];

/// The seed's declarations `n` times under its header.
fn declarations(n: usize) -> String {
    let body = DECLARATIONS
        .find("\n--")
        .expect("the seed's declarations follow a comment");
    let (header, body) = DECLARATIONS.split_at(body);
    format!("{header}{}", body.repeat(n))
}

fn parse(text: &str) {
    drop(parse_module(text));
}

fn parse_and_validate(text: &str) {
    let parse = parse_module(text);
    validate(&parse, Tool::Elm(Source::Application, Compat::V0_19_1));
}

/// The fastest of runs taking at least a second together, and at least
/// five.
fn fastest(run: fn(&str), text: &str) -> Duration {
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
            "| {:<24} | {:>9} | {:>9} | {:>9} | {:>9} | {:>5} |",
            "shape", "size", "time", "2× size", "time", "ratio"
        );
        println!(
            "|{:-<26}|{:->11}|{:->11}|{:->11}|{:->11}|{:->7}|",
            "", "", "", "", "", ""
        );
        let mut failed = Vec::new();
        for shape in SHAPES {
            let (small, large) = ((shape.input)(shape.size), (shape.input)(2 * shape.size));
            let (t1, t2) = (fastest(shape.run, &small), fastest(shape.run, &large));
            let ratio = t2.as_secs_f64() / t1.as_secs_f64();
            println!(
                "| {:<24} | {:>9} | {:>9} | {:>9} | {:>9} | {ratio:>5.2} |",
                shape.name,
                kib(small.len()),
                ms(t1),
                kib(large.len()),
                ms(t2),
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
