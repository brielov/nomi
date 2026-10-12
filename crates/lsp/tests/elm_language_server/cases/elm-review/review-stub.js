#!/usr/bin/env node
// A stand-in for elm-review, which isn't pinned: its JSON report for
// src/Main.elm, whatever it's asked.
const report = {
  type: "review-errors",
  errors: [
    {
      path: "src/Main.elm",
      errors: [
        {
          rule: "NoUnused.Variables",
          ruleLink: "https://package.elm-lang.org/packages/jfmengels/elm-review-unused/latest/NoUnused-Variables",
          message: "Top-level variable `unused` is not used",
          details: ["You should either use this value somewhere, or remove it at the location I pointed at."],
          region: { start: { line: 6, column: 1 }, end: { line: 6, column: 7 } },
          fix: [{ range: { start: { line: 6, column: 1 }, end: { line: 10, column: 1 } }, string: "" }],
        },
        {
          rule: "Simplify",
          ruleLink: "https://package.elm-lang.org/packages/jfmengels/elm-review-simplify/latest/Simplify",
          message: "Expression can be simplified",
          details: ["The addition of two literals can be computed."],
          region: { start: { line: 13, column: 27 }, end: { line: 13, column: 32 } },
        },
        {
          rule: "NoDebug.Log",
          ruleLink: "https://package.elm-lang.org/packages/jfmengels/elm-review-debug/latest/NoDebug-Log",
          message: "Remove the use of `Debug.log` before shipping to production",
          details: ["Suppressed."],
          region: { start: { line: 13, column: 5 }, end: { line: 13, column: 9 } },
          suppressed: true,
        },
      ],
    },
  ],
};
process.stdout.write(JSON.stringify(report));
process.exit(1);
