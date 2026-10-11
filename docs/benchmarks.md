# Benchmarks

Measured on an Apple M-series laptop (aarch64-apple-darwin, 14 cores),
rustc 1.99.0, release build, unless a section names another machine.
Times are the fastest of at least five runs unless a section says
otherwise.

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

## elm-format (`cargo xtask bench elm-format`)

The release build of nomi, run as `elm-format`, against the pinned
elm-format 0.8.8 (its official build, with its default runtime options)
from process start to exit. Each corpus app is validated whole with
`--validate <app>`, which reads and formats every module and writes
nothing; its largest module is formatted through `--stdin`, as editors
do. Both tools exit alike on every input.

| input                  |   modules |      size | elm-format |      nomi | speedup |
|------------------------|-----------|-----------|------------|-----------|---------|
| exosphere              |       218 |  1940 KiB |   320.0 ms |   46.5 ms |    6.9× |
| exosphere --stdin      |         1 |   191 KiB |   107.6 ms |   15.7 ms |    6.9× |
| liascript              |       189 |  1361 KiB |   264.2 ms |   35.6 ms |    7.4× |
| liascript --stdin      |         1 |   127 KiB |    76.9 ms |   11.4 ms |    6.7× |
| docspell               |       583 |  2478 KiB |   443.6 ms |   85.9 ms |    5.2× |
| docspell --stdin       |         1 |   104 KiB |    78.5 ms |   15.6 ms |    5.0× |
| elm-3d-scene           |        59 |   929 KiB |   126.5 ms |   19.4 ms |    6.5× |
| elm-3d-scene --stdin   |         1 |   132 KiB |    47.3 ms |    3.9 ms |   12.1× |

## elm (`cargo xtask bench elm`)

Measured on a laptop with an AMD Ryzen 7 5800H (x86_64-unknown-linux-gnu,
8 cores, 16 threads, 15 GB), rustc 1.99.0. The release build of nomi, run
as `elm`, against the pinned elm 0.19.1 (its official build, with its
default runtime options), building each corpus app's entry with `elm make`
from process start to exit. A cold build starts without `elm-stuff` or any
built dependency: elm's `artifacts.dat` files are removed from its
`ELM_HOME` and nomi's artifact cache is emptied. A warm build follows it
with nothing changed. Both tools run offline over the same package sources
and write the same JavaScript. Wall time is the fastest of three runs; CPU
time (user and system) and peak memory are the least of any run, as
`/usr/bin/time` reports them.

| build             |    elm |   nomi | speedup | elm CPU | nomi CPU | elm memory | nomi memory |
|-------------------|--------|--------|---------|---------|----------|------------|-------------|
| exosphere cold    | 2.23 s | 2.92 s |   0.76× |  8.97 s |   8.06 s |   2361 MiB |    1605 MiB |
| exosphere warm    | 0.41 s | 0.27 s |   1.50× |  0.57 s |   0.71 s |    535 MiB |     188 MiB |
| liascript cold    | 1.03 s | 0.93 s |   1.10× |  3.57 s |   2.27 s |   2127 MiB |     267 MiB |
| liascript warm    | 0.28 s | 0.19 s |   1.51× |  0.46 s |   0.45 s |    369 MiB |      96 MiB |
| docspell cold     | 1.76 s | 2.44 s |   0.72× |  7.29 s |   7.49 s |   2325 MiB |    1531 MiB |
| docspell warm     | 0.44 s | 0.34 s |   1.27× |  0.80 s |   0.95 s |    583 MiB |     151 MiB |
| elm-3d-scene cold | 1.04 s | 0.76 s |   1.37× |  3.01 s |   1.18 s |   2108 MiB |     117 MiB |
| elm-3d-scene warm | 0.19 s | 0.14 s |   1.37× |  0.18 s |   0.18 s |    257 MiB |      73 MiB |

M8's bounds hold: no cold build takes more than 1.5 times elm's time, and
every warm build is faster. Cold builds of the two largest apps are still
slower, and warm builds spend more CPU time than elm's, which M8b takes on.

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
