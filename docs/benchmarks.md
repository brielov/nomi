# Benchmarks

Measured on an Apple M-series laptop (aarch64-apple-darwin, 14 cores),
rustc 1.99.0, release build. Times are the fastest of at least five runs.

## Syntax (`cargo xtask bench syntax`)

Each shape is timed at a size and at twice that size. The run fails when
the larger input takes three times as long as the smaller or more.

| shape                    |      size |      time |   2× size |      time | ratio |
|--------------------------|-----------|-----------|-----------|-----------|-------|
| declarations             |   186 KiB |   3.51 ms |   372 KiB |   7.08 ms |  2.02 |
| declarations, validated  |   186 KiB |  14.38 ms |   372 KiB |  28.90 ms |  2.01 |
| nested parentheses       |    19 KiB |   1.52 ms |    39 KiB |   3.03 ms |  2.00 |
| operator chain           |   292 KiB |  21.73 ms |   585 KiB |  43.86 ms |  2.02 |
| list                     |   488 KiB |  11.78 ms |   976 KiB |  25.04 ms |  2.13 |
| shader                   |   390 KiB |  41.47 ms |   781 KiB |  83.03 ms |  2.00 |

"Validated" adds `validate` for elm 0.19.1 applications; the shader shape
always does, since shaders are checked only there.

## Format (`cargo xtask bench format`)

Each shape is timed at a size, twice it and four times it. The run fails
when doubling the input takes three times as long or more; the ratio is
the larger of the two.

| shape            |      size |      time |   2× size |      time |   4× size |      time | ratio |
|------------------|-----------|-----------|-----------|-----------|-----------|-----------|-------|
| module           |   132 KiB |  16.63 ms |   260 KiB |  34.04 ms |   517 KiB |  68.92 ms |  2.05 |
| nested calls     |     9 KiB |   4.45 ms |    19 KiB |   9.29 ms |    39 KiB |  20.01 ms |  2.15 |
| nested records   |    19 KiB |   6.90 ms |    39 KiB |  14.12 ms |    78 KiB |  31.29 ms |  2.22 |
| nested lambdas   |    14 KiB |   3.95 ms |    29 KiB |   8.13 ms |    58 KiB |  16.69 ms |  2.06 |
| else ifs         |    41 KiB |   7.35 ms |    83 KiB |  14.92 ms |   166 KiB |  30.81 ms |  2.07 |
| operator chain   |    78 KiB |  29.03 ms |   156 KiB |  60.96 ms |   312 KiB | 121.99 ms |  2.10 |

"Module" repeats the declarations of style-elements' `Element` under its
header. Shapes whose output grows faster than their input, as nested
`case`s and `let`s do with their indentation, are left out.

## Fuzzing (`cargo xtask fuzz syntax`)

A libFuzzer campaign over `nomi_syntax::check_invariants`: every entry point
gives back its text, and the errors of parsing and of validating for four
tools lie in bounds, on character boundaries, in order. Built on stable
with libFuzzer's coverage instrumentation, debug assertions and overflow
checks, and no sanitizer.

| date       | runs      | jobs | wall  | crashes | coverage |
|------------|-----------|------|-------|---------|----------|
| 2026-10-07 | 1,000,073 |   14 | 547 s |       0 |     3182 |

The campaign started from the eight mutation seeds and an empty corpus.
Each job ran about 150 inputs a second, about 2.1k altogether.
