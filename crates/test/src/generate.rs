//! `Test.Generated.Main`, laid out as `Generate.js` lays it out
//! (`makeTestFileBody`, `makeOptsCode`): the test modules imported, the
//! run's options, and each module's tests.

/// The reporter a run asks for (`generateElmReportVariant`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Report {
    Console { color: bool },
    Json,
    JUnit,
}

/// What the runner's `run` takes besides the tests.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Options {
    pub runs: u32,
    pub report: Report,
    pub seed: u64,
    pub processes: usize,
    pub globs: Vec<String>,
    pub paths: Vec<String>,
}

/// The generated module's name.
pub const MAIN: &str = "Test.Generated.Main";

/// `Test.Generated.Main`'s text: `imports` imported, and each module of
/// `tests` with its tests. elm-test passes every exposed value through
/// `check`, which its patch of the JavaScript answers; nomi lists only
/// the values whose type is `Test`, so it passes each in `Just`.
pub fn main_module(
    options: &Options,
    imports: &[String],
    tests: &[(String, Vec<String>)],
) -> String {
    let imports: Vec<String> = imports.iter().map(|m| format!("import {m}")).collect();
    let tuples: Vec<String> = (tests.iter())
        .map(|(module, tests)| {
            let list: Vec<String> = tests.iter().map(|t| format!("Just {module}.{t}")).collect();
            format!(
                "( \"{module}\"\n, {}\n)",
                indent_all_but_first_line("  ", &make_list(&list))
            )
        })
        .collect();
    let body = format!(
        "\n{}\n\nimport Test.Reporter.Reporter exposing (Report(..))\nimport Console.Text exposing (UseColor(..))\nimport Test.Runner.Node\nimport Test\n\nmain : Test.Runner.Node.TestProgram\nmain =\n    Test.Runner.Node.run\n        {}\n        {}\n  ",
        imports.join("\n"),
        indent_all_but_first_line("        ", &options_code(options)),
        indent_all_but_first_line("        ", &make_list(&tuples)),
    );
    format!("module {MAIN} exposing (main)\n\n{}", body.trim())
}

/// `makeOptsCode`.
fn options_code(options: &Options) -> String {
    let report = match options.report {
        Report::Json => "JsonReport",
        Report::JUnit => "JUnitReport",
        Report::Console { color: true } => "ConsoleReport UseColor",
        Report::Console { color: false } => "ConsoleReport Monochrome",
    };
    let strings = |list: &[String]| {
        let list: Vec<String> = list.iter().map(|s| elm_string(s)).collect();
        indent_all_but_first_line("    ", &make_list(&list))
    };
    format!(
        "{{ runs = {}\n, report = {report}\n, seed = {}\n, processes = {}\n, globs =\n    {}\n, paths =\n    {}\n}}",
        options.runs,
        options.seed,
        options.processes,
        strings(&options.globs),
        strings(&options.paths),
    )
}

/// `makeList`.
fn make_list(parts: &[String]) -> String {
    if parts.is_empty() {
        return "[]".to_owned();
    }
    let list: Vec<String> = (parts.iter().enumerate())
        .map(|(i, part)| {
            let comma = if i == 0 { "" } else { ", " };
            format!("{comma}{}", indent_all_but_first_line("  ", part))
        })
        .collect();
    format!("[ {}\n]", list.join("\n"))
}

/// `indentAllButFirstLine`.
fn indent_all_but_first_line(indent: &str, string: &str) -> String {
    (string.split('\n').enumerate())
        .map(|(i, line)| {
            if i == 0 {
                line.to_owned()
            } else {
                format!("{indent}{line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// `makeElmString`.
fn elm_string(string: &str) -> String {
    let escaped = string
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r");
    format!("\"{escaped}\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `Main` elm-test 0.19.3-0 generated for three of the failing
    /// and passing suites' modules, with `check` where nomi has `Just`.
    #[test]
    fn main_is_laid_out_as_elm_tests() {
        let expected = r#"module Test.Generated.Main exposing (main)

import Failing.Fuzz
import Failing.One
import Passing.Several

import Test.Reporter.Reporter exposing (Report(..))
import Console.Text exposing (UseColor(..))
import Test.Runner.Node
import Test

main : Test.Runner.Node.TestProgram
main =
    Test.Runner.Node.run
        { runs = 100
        , report = JUnitReport
        , seed = 12345
        , processes = 16
        , globs =
            []
        , paths =
            [ "/p/tests/Failing/Fuzz.elm"
            , "/p/tests/Failing/One.elm"
            , "/p/tests/Passing/Several.elm"
            ]
        }
        [ ( "Failing.Fuzz"
          , [ Just Failing.Fuzz.withoutNums
            , Just Failing.Fuzz.testWithoutNums
            ]
          )
        , ( "Failing.One"
          , [ Just Failing.One.suite
            ]
          )
        , ( "Passing.Several"
          , [ Just Passing.Several.testEqual
            ]
          )
        ]"#;
        let modules = ["Failing.Fuzz", "Failing.One", "Passing.Several"];
        let options = Options {
            runs: 100,
            report: Report::JUnit,
            seed: 12345,
            processes: 16,
            globs: Vec::new(),
            paths: (modules.iter())
                .map(|m| format!("/p/tests/{}.elm", m.replace('.', "/")))
                .collect(),
        };
        let tests = vec![
            (
                modules[0].to_owned(),
                vec!["withoutNums".to_owned(), "testWithoutNums".to_owned()],
            ),
            (modules[1].to_owned(), vec!["suite".to_owned()]),
            (modules[2].to_owned(), vec!["testEqual".to_owned()]),
        ];
        let imports: Vec<String> = modules.iter().map(|m| (*m).to_owned()).collect();
        assert_eq!(main_module(&options, &imports, &tests), expected);
        // No tests, and a glob with what a string must escape.
        let options = Options {
            report: Report::Console { color: false },
            globs: vec!["a\"b\\c".to_owned()],
            ..options
        };
        let main = main_module(&options, &imports, &[]);
        assert!(
            main.contains("        , report = ConsoleReport Monochrome\n"),
            "{main}"
        );
        assert!(
            main.contains("        , globs =\n            [ \"a\\\"b\\\\c\"\n            ]\n"),
            "{main}"
        );
        assert!(main.ends_with("        }\n        []"), "{main}");
    }
}
