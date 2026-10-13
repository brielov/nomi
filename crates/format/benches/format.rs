//! `cargo xtask bench format`: how long formatting takes on inputs of each
//! shape, at three sizes, each twice the one before. Each must take less
//! than three times as long as the one before, or the run fails:
//! formatting scales linearly. Times are the fastest of several runs.

use nomi_testkit::bench::{SCALING, fail_if_slow, fastest, kib, ms, ratio};

/// A real module: style-elements' `Element`, from elm-format's tests.
const ELEMENT: &str = include_str!(
    "../tests/format_fixtures/inputs/elm-format/tests/test-files/good/Elm-0.18/mdgriffith/style-elements/Element.elm"
);

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

fn main() {
    // Deep trees need the stack the shell gives core work.
    let failed = nomi_base::on_core_stack(|| {
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
            let times = inputs.each_ref().map(|text| fastest(|| run(text)));
            let ratio = ratio(&times);
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
    fail_if_slow(&failed);
}
