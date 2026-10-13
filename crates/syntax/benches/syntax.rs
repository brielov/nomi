//! `cargo xtask bench syntax`: how long parsing and validation take on
//! inputs of each shape, at two sizes. The second is twice the first, and
//! must take less than three times as long, or the run fails: parsing
//! scales linearly. Times are the fastest of several runs.

use nomi_base::Compat;
use nomi_syntax::{Source, Tool, parse_module, validate};
use nomi_testkit::bench::{SCALING, fail_if_slow, fastest, kib, ms, ratio};

const DECLARATIONS: &str = include_str!("../tests/syntax_mutations/seeds/Declarations.elm");

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

fn main() {
    // Deep trees need the stack the shell gives core work.
    let failed = nomi_base::on_core_stack(|| {
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
            let t1 = fastest(|| (shape.run)(&small));
            let t2 = fastest(|| (shape.run)(&large));
            let ratio = ratio(&[t1, t2]);
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
    fail_if_slow(&failed);
}
