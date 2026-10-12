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

## Scoreboard (`cargo xtask bench`)

nomi's release build against the pinned upstream tools on every workload
they share (plan M8b), written here by `cargo xtask bench`; `bench elm` and
`bench elm-format` print their part. CPU time is user plus system time and
memory is peak resident memory, both as `/usr/bin/time` reports them; it
reads CPU time in hundredths of a second.

<!-- scoreboard:start -->

Measured on AMD Ryzen 7 5800H with Radeon Graphics (x86_64-linux, 16 threads, 16 GB), rustc 1.99.0. nomi is at or under upstream in 186 of 204 cells; a cell where it isn't is marked ✗.

### elm make

nomi, run as `elm`, against each pinned elm (its official build, with its default runtime options), building each corpus app's entries from process start to exit, in dev and with `--optimize`. A cold build starts without `elm-stuff` or any built dependency: elm's `artifacts.dat` files are removed from its `ELM_HOME`, and nomi's artifact cache is emptied. A warm build follows it with nothing changed. Both tools run offline over the same package sources and write the same JavaScript. The best of 3 runs counts for each measure.

| workload | elm | nomi | speedup | elm CPU | nomi CPU | elm memory | nomi memory |
|---|--:|--:|--:|--:|--:|--:|--:|
| exosphere 0.19.1 cold | 2.34 s | 1.93 s | 1.22× | 9.36 s | 6.26 s | 2380 MiB | 954 MiB |
| exosphere 0.19.1 warm | 419.4 ms | 259.2 ms | 1.62× | 570.0 ms | 750.0 ms ✗ | 534 MiB | 229 MiB |
| exosphere 0.19.1 `--optimize` cold | 2.35 s | 1.99 s | 1.18× | 9.27 s | 6.14 s | 2376 MiB | 949 MiB |
| exosphere 0.19.1 `--optimize` warm | 427.2 ms | 263.3 ms | 1.62× | 580.0 ms | 760.0 ms ✗ | 535 MiB | 224 MiB |
| liascript 0.19.1 cold | 1.09 s | 610.4 ms | 1.78× | 3.72 s | 2.10 s | 2128 MiB | 243 MiB |
| liascript 0.19.1 warm | 278.8 ms | 161.0 ms | 1.73× | 460.0 ms | 450.0 ms | 368 MiB | 104 MiB |
| liascript 0.19.1 `--optimize` cold | 1.10 s | 602.0 ms | 1.82× | 3.73 s | 2.05 s | 2132 MiB | 238 MiB |
| liascript 0.19.1 `--optimize` warm | 289.9 ms | 164.0 ms | 1.77× | 460.0 ms | 460.0 ms | 368 MiB | 105 MiB |
| docspell 0.19.1 cold | 1.80 s | 1.48 s | 1.22× | 7.35 s | 5.68 s | 2316 MiB | 990 MiB |
| docspell 0.19.1 warm | 449.9 ms | 305.8 ms | 1.47× | 810.0 ms | 930.0 ms ✗ | 579 MiB | 167 MiB |
| docspell 0.19.1 `--optimize` cold | 1.80 s | 1.47 s | 1.22× | 7.24 s | 5.66 s | 2315 MiB | 992 MiB |
| docspell 0.19.1 `--optimize` warm | 460.3 ms | 309.9 ms | 1.49× | 810.0 ms | 940.0 ms ✗ | 581 MiB | 163 MiB |
| elm-3d-scene 0.19.1 cold | 1.06 s | 322.9 ms | 3.28× | 3.07 s | 1.25 s | 2109 MiB | 162 MiB |
| elm-3d-scene 0.19.1 warm | 190.3 ms | 118.5 ms | 1.61× | 180.0 ms | 170.0 ms | 257 MiB | 81 MiB |
| elm-3d-scene 0.19.1 `--optimize` cold | 1.06 s | 323.7 ms | 3.27× | 3.08 s | 1.26 s | 2112 MiB | 167 MiB |
| elm-3d-scene 0.19.1 `--optimize` warm | 189.2 ms | 118.6 ms | 1.60× | 180.0 ms | 170.0 ms | 257 MiB | 76 MiB |
| exosphere 0.19.2 cold | 2.73 s | 1.86 s | 1.46× | 8.52 s | 6.16 s | 2693 MiB | 944 MiB |
| exosphere 0.19.2 warm | 403.1 ms | 253.3 ms | 1.59× | 550.0 ms | 740.0 ms ✗ | 518 MiB | 218 MiB |
| exosphere 0.19.2 `--optimize` cold | 2.71 s | 1.87 s | 1.45× | 8.60 s | 6.08 s | 2695 MiB | 944 MiB |
| exosphere 0.19.2 `--optimize` warm | 403.9 ms | 256.4 ms | 1.58× | 550.0 ms | 750.0 ms ✗ | 519 MiB | 223 MiB |
| liascript 0.19.2 cold | 1.01 s | 596.5 ms | 1.69× | 3.29 s | 2.01 s | 2149 MiB | 237 MiB |
| liascript 0.19.2 warm | 273.1 ms | 161.4 ms | 1.69× | 440.0 ms | 450.0 ms ✗ | 360 MiB | 104 MiB |
| liascript 0.19.2 `--optimize` cold | 1.02 s | 599.3 ms | 1.69× | 3.34 s | 2.08 s | 2144 MiB | 247 MiB |
| liascript 0.19.2 `--optimize` warm | 267.7 ms | 162.7 ms | 1.65× | 450.0 ms | 450.0 ms | 360 MiB | 104 MiB |
| docspell 0.19.2 cold | 1.80 s | 1.46 s | 1.24× | 6.50 s | 5.56 s | 2428 MiB | 1000 MiB |
| docspell 0.19.2 warm | 424.4 ms | 308.7 ms | 1.37× | 770.0 ms | 950.0 ms ✗ | 566 MiB | 168 MiB |
| docspell 0.19.2 `--optimize` cold | 1.83 s | 1.48 s | 1.23× | 6.42 s | 5.68 s | 2433 MiB | 993 MiB |
| docspell 0.19.2 `--optimize` warm | 429.4 ms | 310.2 ms | 1.38× | 780.0 ms | 950.0 ms ✗ | 568 MiB | 166 MiB |
| elm-3d-scene 0.19.2 cold | 1.09 s | 323.0 ms | 3.38× | 2.84 s | 1.23 s | 2110 MiB | 160 MiB |
| elm-3d-scene 0.19.2 warm | 179.8 ms | 120.8 ms | 1.49× | 180.0 ms | 170.0 ms | 254 MiB | 78 MiB |
| elm-3d-scene 0.19.2 `--optimize` cold | 1.09 s | 322.2 ms | 3.37× | 2.85 s | 1.24 s | 2115 MiB | 163 MiB |
| elm-3d-scene 0.19.2 `--optimize` warm | 181.9 ms | 119.7 ms | 1.52× | 180.0 ms | 170.0 ms | 254 MiB | 77 MiB |
| exosphere 0.19.3 cold | 1.79 s | 1.92 s ✗ | 0.93× | 7.36 s | 6.34 s | 2237 MiB | 952 MiB |
| exosphere 0.19.3 warm | 426.3 ms | 256.6 ms | 1.66× | 630.0 ms | 770.0 ms ✗ | 570 MiB | 233 MiB |
| exosphere 0.19.3 `--optimize` cold | 1.77 s | 1.95 s ✗ | 0.91× | 7.26 s | 6.19 s | 2234 MiB | 948 MiB |
| exosphere 0.19.3 `--optimize` warm | 440.8 ms | 257.2 ms | 1.71× | 630.0 ms | 760.0 ms ✗ | 571 MiB | 228 MiB |
| liascript 0.19.3 cold | 1.06 s | 604.8 ms | 1.75× | 3.32 s | 1.98 s | 2150 MiB | 245 MiB |
| liascript 0.19.3 warm | 286.0 ms | 159.9 ms | 1.79× | 480.0 ms | 450.0 ms | 392 MiB | 102 MiB |
| liascript 0.19.3 `--optimize` cold | 1.06 s | 605.3 ms | 1.75× | 3.29 s | 2.09 s | 2153 MiB | 250 MiB |
| liascript 0.19.3 `--optimize` warm | 285.4 ms | 159.2 ms | 1.79× | 460.0 ms | 440.0 ms | 392 MiB | 101 MiB |
| docspell 0.19.3 cold | 2.03 s | 1.47 s | 1.38× | 6.45 s | 5.87 s | 2546 MiB | 1005 MiB |
| docspell 0.19.3 warm | 445.1 ms | 297.6 ms | 1.50× | 840.0 ms | 940.0 ms ✗ | 618 MiB | 167 MiB |
| docspell 0.19.3 `--optimize` cold | 2.05 s | 1.47 s | 1.39× | 6.56 s | 5.85 s | 2545 MiB | 989 MiB |
| docspell 0.19.3 `--optimize` warm | 455.8 ms | 299.4 ms | 1.52× | 850.0 ms | 940.0 ms ✗ | 621 MiB | 164 MiB |
| elm-3d-scene 0.19.3 cold | 1.10 s | 318.5 ms | 3.44× | 2.80 s | 1.23 s | 2104 MiB | 156 MiB |
| elm-3d-scene 0.19.3 warm | 206.2 ms | 119.5 ms | 1.73× | 210.0 ms | 170.0 ms | 285 MiB | 74 MiB |
| elm-3d-scene 0.19.3 `--optimize` cold | 1.09 s | 319.4 ms | 3.43× | 2.79 s | 1.24 s | 2106 MiB | 162 MiB |
| elm-3d-scene 0.19.3 `--optimize` warm | 211.5 ms | 119.6 ms | 1.77× | 210.0 ms | 170.0 ms | 285 MiB | 73 MiB |

### elm make --docs

As for `elm make`, documenting packages with `--docs`: the core library, three widely used packages, the largest corpus package by modules and one of the largest by bytes, with the same `docs.json` from both. A package's `elm.json` admits a range of releases, and nomi builds it as the newest of them does, so only the newest elm is measured. The best of 3 runs counts for each measure.

| workload | elm | nomi | speedup | elm CPU | nomi CPU | elm memory | nomi memory |
|---|--:|--:|--:|--:|--:|--:|--:|
| elm/core 1.0.5, 0.19.3 cold | 83.7 ms | 25.6 ms | 3.27× | 100.0 ms | 40.0 ms | 115 MiB | 27 MiB |
| elm/core 1.0.5, 0.19.3 warm | 79.5 ms | 26.6 ms | 2.99× | 100.0 ms | 40.0 ms | 106 MiB | 27 MiB |
| mdgriffith/elm-ui 1.1.8, 0.19.3 cold | 291.1 ms | 124.1 ms | 2.35× | 470.0 ms | 260.0 ms | 440 MiB | 67 MiB |
| mdgriffith/elm-ui 1.1.8, 0.19.3 warm | 96.4 ms | 106.9 ms ✗ | 0.90× | 100.0 ms | 140.0 ms ✗ | 126 MiB | 54 MiB |
| rtfeldman/elm-css 18.0.0, 0.19.3 cold | 326.1 ms | 119.8 ms | 2.72× | 600.0 ms | 310.0 ms | 545 MiB | 82 MiB |
| rtfeldman/elm-css 18.0.0, 0.19.3 warm | 204.1 ms | 106.0 ms | 1.92× | 270.0 ms | 180.0 ms | 283 MiB | 68 MiB |
| dillonkearns/elm-graphql 5.0.12, 0.19.3 cold | 190.6 ms | 78.5 ms | 2.43× | 400.0 ms | 200.0 ms | 341 MiB | 53 MiB |
| dillonkearns/elm-graphql 5.0.12, 0.19.3 warm | 63.6 ms | 28.8 ms | 2.21× | 60.0 ms | 50.0 ms | 77 MiB | 31 MiB |
| finos/morphir-elm 22.0.2, 0.19.3 cold | 42.49 s | 397.6 ms | 106.86× | 46.23 s | 2.68 s | 2202 MiB | 292 MiB |
| finos/morphir-elm 22.0.2, 0.19.3 warm | 262.3 ms | 239.1 ms | 1.10× | 530.0 ms | 860.0 ms ✗ | 455 MiB | 195 MiB |
| icidasset/elm-material-icons 11.0.0, 0.19.3 cold | 1.17 s | 560.5 ms | 2.09× | 2.32 s | 1.41 s | 2216 MiB | 667 MiB |
| icidasset/elm-material-icons 11.0.0, 0.19.3 warm | 1.09 s | 528.6 ms | 2.06× | 2.22 s | 1.33 s | 2158 MiB | 650 MiB |

### elm-format

nomi, run as `elm-format`, against the pinned elm-format 0.8.8 (its official build, with its default runtime options), from process start to exit. Each corpus app is validated whole with `--validate`, which reads and formats every module and writes nothing; its largest module is formatted through `--stdin`, as editors do, with the same output from both. The best of 5 runs counts for each measure.

| workload | elm-format | nomi | speedup | elm-format CPU | nomi CPU | elm-format memory | nomi memory |
|---|--:|--:|--:|--:|--:|--:|--:|
| exosphere `--validate`, 218 modules, 1940 KiB | 1.17 s | 57.0 ms | 20.60× | 13.32 s | 430.0 ms | 122 MiB | 42 MiB |
| exosphere `--stdin`, 191 KiB | 154.2 ms | 23.2 ms | 6.63× | 210.0 ms | 10.0 ms | 28 MiB | 14 MiB |
| liascript `--validate`, 189 modules, 1361 KiB | 966.4 ms | 38.5 ms | 25.12× | 11.31 s | 400.0 ms | 127 MiB | 42 MiB |
| liascript `--stdin`, 127 KiB | 112.9 ms | 15.8 ms | 7.16× | 130.0 ms | 0.0 ms | 21 MiB | 12 MiB |
| docspell `--validate`, 583 modules, 2478 KiB | 1.90 s | 89.3 ms | 21.33× | 23.01 s | 730.0 ms | 120 MiB | 41 MiB |
| docspell `--stdin`, 104 KiB | 113.5 ms | 22.0 ms | 5.15× | 130.0 ms | 10.0 ms | 21 MiB | 13 MiB |
| elm-3d-scene `--validate`, 59 modules, 929 KiB | 473.5 ms | 25.2 ms | 18.77× | 4.68 s | 180.0 ms | 111 MiB | 37 MiB |
| elm-3d-scene `--stdin`, 132 KiB | 63.2 ms | 4.9 ms | 12.93× | 60.0 ms | 0.0 ms | 20 MiB | 7 MiB |

<!-- scoreboard:end -->

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
