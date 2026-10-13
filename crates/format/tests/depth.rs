//! Nesting as deep as generated code goes formats on the stack the shell
//! gives threads that run core work: lowering, rendering and the doc
//! comments' Markdown recurse with what they read, and grow the stack as
//! the parser does.

/// How deeply each shape nests.
const DEPTH: usize = 5_000;

/// `x` defined as `body`.
fn module(body: &str) -> String {
    format!("module A exposing (x)\n\n\nx =\n    {body}\n")
}

/// `open`, then `1`, then `close`, each `DEPTH` times.
fn nested(open: &str, close: &str) -> String {
    module(&format!("{}1{}", open.repeat(DEPTH), close.repeat(DEPTH)))
}

#[test]
fn deep_nesting_formats() {
    let shapes = [
        ("parentheses", nested("(", ")")),
        ("lists", nested("[ ", " ]")),
        ("tuples", nested("( 1, ", " )")),
        ("records", nested("{ a = ", " }")),
        ("calls", nested("f (", ")")),
        ("lambdas", nested("\\a -> ", "")),
        ("else ifs", nested("if a then 1 else ", "")),
        ("negations", nested("-(", ")")),
        ("left pipes", nested("f <| ", "")),
        (
            "types",
            format!(
                "module A exposing (x)\n\n\nx : {}Int{}\nx =\n    1\n",
                "List (".repeat(DEPTH),
                ")".repeat(DEPTH)
            ),
        ),
        (
            "function types",
            format!(
                "module A exposing (x)\n\n\nx : {}Int\nx =\n    1\n",
                "Int -> ".repeat(DEPTH)
            ),
        ),
        (
            "patterns",
            format!(
                "module A exposing (x)\n\n\nx {}a{} =\n    1\n",
                "(".repeat(DEPTH),
                ")".repeat(DEPTH)
            ),
        ),
        (
            "cons patterns",
            format!(
                "module A exposing (x)\n\n\nx ({}a) =\n    1\n",
                "a :: ".repeat(DEPTH)
            ),
        ),
        (
            "block quotes",
            format!(
                "module A exposing (x)\n\n{{-| {}x\n-}}\n\n\nx =\n    1\n",
                "> ".repeat(DEPTH)
            ),
        ),
    ];
    nomi_base::on_core_stack(|| {
        for (name, text) in shapes {
            assert!(nomi_format::format(&text).is_some(), "{name}");
        }
    });
}
