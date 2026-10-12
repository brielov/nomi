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

Measured on AMD Ryzen 7 5800H with Radeon Graphics (x86_64-linux, 16 threads, 16 GB), rustc 1.99.0. nomi is at or under upstream in 204 of 204 cells; a cell where it isn't is marked ✗.

### elm make

nomi, run as `elm`, against each pinned elm (its official build, with its default runtime options), building each corpus app's entries from process start to exit, in dev and with `--optimize`. A cold build starts without `elm-stuff` or any built dependency: elm's `artifacts.dat` files are removed from its `ELM_HOME`, and nomi's artifact cache is emptied. A warm build follows it with nothing changed. Both tools run offline over the same package sources and write the same JavaScript. The best of 3 runs counts for each measure.

| workload | elm | nomi | speedup | elm CPU | nomi CPU | elm memory | nomi memory |
|---|--:|--:|--:|--:|--:|--:|--:|
| exosphere 0.19.1 cold | 2.30 s | 1.43 s | 1.61× | 9.02 s | 5.17 s | 2374 MiB | 691 MiB |
| exosphere 0.19.1 warm | 414.8 ms | 137.7 ms | 3.01× | 570.0 ms | 370.0 ms | 534 MiB | 116 MiB |
| exosphere 0.19.1 `--optimize` cold | 2.29 s | 1.42 s | 1.61× | 9.14 s | 5.11 s | 2375 MiB | 699 MiB |
| exosphere 0.19.1 `--optimize` warm | 425.2 ms | 139.0 ms | 3.06× | 590.0 ms | 380.0 ms | 535 MiB | 113 MiB |
| liascript 0.19.1 cold | 1.04 s | 502.5 ms | 2.07× | 3.63 s | 1.84 s | 2125 MiB | 205 MiB |
| liascript 0.19.1 warm | 279.0 ms | 103.9 ms | 2.69× | 450.0 ms | 280.0 ms | 367 MiB | 80 MiB |
| liascript 0.19.1 `--optimize` cold | 1.05 s | 514.7 ms | 2.04× | 3.64 s | 1.80 s | 2134 MiB | 211 MiB |
| liascript 0.19.1 `--optimize` warm | 288.2 ms | 110.2 ms | 2.62× | 470.0 ms | 290.0 ms | 368 MiB | 83 MiB |
| docspell 0.19.1 cold | 1.77 s | 1.08 s | 1.65× | 7.27 s | 4.70 s | 2317 MiB | 737 MiB |
| docspell 0.19.1 warm | 442.9 ms | 170.5 ms | 2.60× | 790.0 ms | 550.0 ms | 579 MiB | 117 MiB |
| docspell 0.19.1 `--optimize` cold | 1.78 s | 1.09 s | 1.63× | 7.18 s | 4.69 s | 2314 MiB | 720 MiB |
| docspell 0.19.1 `--optimize` warm | 459.9 ms | 171.4 ms | 2.68× | 800.0 ms | 530.0 ms | 582 MiB | 114 MiB |
| elm-3d-scene 0.19.1 cold | 1.04 s | 303.9 ms | 3.43× | 3.01 s | 1.20 s | 2109 MiB | 148 MiB |
| elm-3d-scene 0.19.1 warm | 190.3 ms | 87.8 ms | 2.17× | 180.0 ms | 120.0 ms | 257 MiB | 59 MiB |
| elm-3d-scene 0.19.1 `--optimize` cold | 1.05 s | 318.2 ms | 3.31× | 3.04 s | 1.16 s | 2104 MiB | 143 MiB |
| elm-3d-scene 0.19.1 `--optimize` warm | 188.3 ms | 90.1 ms | 2.09× | 180.0 ms | 120.0 ms | 257 MiB | 60 MiB |
| exosphere 0.19.2 cold | 2.67 s | 1.41 s | 1.89× | 8.50 s | 5.24 s | 2681 MiB | 706 MiB |
| exosphere 0.19.2 warm | 394.7 ms | 140.3 ms | 2.81× | 540.0 ms | 380.0 ms | 518 MiB | 117 MiB |
| exosphere 0.19.2 `--optimize` cold | 2.71 s | 1.41 s | 1.92× | 8.58 s | 5.22 s | 2705 MiB | 700 MiB |
| exosphere 0.19.2 `--optimize` warm | 399.3 ms | 140.0 ms | 2.85× | 550.0 ms | 380.0 ms | 519 MiB | 111 MiB |
| liascript 0.19.2 cold | 996.6 ms | 501.3 ms | 1.99× | 3.29 s | 1.82 s | 2140 MiB | 213 MiB |
| liascript 0.19.2 warm | 265.5 ms | 106.9 ms | 2.48× | 440.0 ms | 280.0 ms | 360 MiB | 83 MiB |
| liascript 0.19.2 `--optimize` cold | 1.01 s | 505.0 ms | 1.99× | 3.30 s | 1.85 s | 2145 MiB | 214 MiB |
| liascript 0.19.2 `--optimize` warm | 266.4 ms | 107.0 ms | 2.49× | 430.0 ms | 290.0 ms | 360 MiB | 82 MiB |
| docspell 0.19.2 cold | 1.78 s | 1.08 s | 1.65× | 6.44 s | 4.71 s | 2427 MiB | 732 MiB |
| docspell 0.19.2 warm | 414.0 ms | 167.0 ms | 2.48× | 770.0 ms | 540.0 ms | 566 MiB | 117 MiB |
| docspell 0.19.2 `--optimize` cold | 1.80 s | 1.08 s | 1.66× | 6.51 s | 4.71 s | 2429 MiB | 730 MiB |
| docspell 0.19.2 `--optimize` warm | 428.2 ms | 169.0 ms | 2.53× | 780.0 ms | 540.0 ms | 568 MiB | 113 MiB |
| elm-3d-scene 0.19.2 cold | 1.06 s | 304.1 ms | 3.48× | 2.77 s | 1.18 s | 2117 MiB | 146 MiB |
| elm-3d-scene 0.19.2 warm | 179.8 ms | 86.8 ms | 2.07× | 180.0 ms | 120.0 ms | 253 MiB | 58 MiB |
| elm-3d-scene 0.19.2 `--optimize` cold | 1.07 s | 300.4 ms | 3.56× | 2.78 s | 1.19 s | 2118 MiB | 148 MiB |
| elm-3d-scene 0.19.2 `--optimize` warm | 179.8 ms | 89.1 ms | 2.02× | 170.0 ms | 120.0 ms | 254 MiB | 59 MiB |
| exosphere 0.19.3 cold | 1.71 s | 1.47 s | 1.16× | 7.01 s | 5.29 s | 2224 MiB | 702 MiB |
| exosphere 0.19.3 warm | 420.5 ms | 135.9 ms | 3.09× | 620.0 ms | 370.0 ms | 570 MiB | 114 MiB |
| exosphere 0.19.3 `--optimize` cold | 1.76 s | 1.45 s | 1.21× | 7.14 s | 5.18 s | 2244 MiB | 698 MiB |
| exosphere 0.19.3 `--optimize` warm | 425.0 ms | 135.9 ms | 3.13× | 620.0 ms | 370.0 ms | 571 MiB | 113 MiB |
| liascript 0.19.3 cold | 1.04 s | 512.3 ms | 2.04× | 3.23 s | 1.86 s | 2160 MiB | 207 MiB |
| liascript 0.19.3 warm | 281.6 ms | 105.8 ms | 2.66× | 460.0 ms | 280.0 ms | 392 MiB | 82 MiB |
| liascript 0.19.3 `--optimize` cold | 1.04 s | 507.6 ms | 2.05× | 3.28 s | 1.83 s | 2153 MiB | 213 MiB |
| liascript 0.19.3 `--optimize` warm | 284.8 ms | 109.5 ms | 2.60× | 480.0 ms | 290.0 ms | 392 MiB | 83 MiB |
| docspell 0.19.3 cold | 2.01 s | 1.11 s | 1.81× | 6.40 s | 4.74 s | 2544 MiB | 736 MiB |
| docspell 0.19.3 warm | 437.4 ms | 169.6 ms | 2.58× | 840.0 ms | 530.0 ms | 619 MiB | 115 MiB |
| docspell 0.19.3 `--optimize` cold | 2.03 s | 1.12 s | 1.82× | 6.50 s | 4.75 s | 2547 MiB | 741 MiB |
| docspell 0.19.3 `--optimize` warm | 449.4 ms | 169.7 ms | 2.65× | 840.0 ms | 540.0 ms | 620 MiB | 115 MiB |
| elm-3d-scene 0.19.3 cold | 1.07 s | 305.3 ms | 3.51× | 2.74 s | 1.18 s | 2108 MiB | 148 MiB |
| elm-3d-scene 0.19.3 warm | 207.3 ms | 89.0 ms | 2.33× | 200.0 ms | 120.0 ms | 284 MiB | 58 MiB |
| elm-3d-scene 0.19.3 `--optimize` cold | 1.07 s | 300.0 ms | 3.58× | 2.74 s | 1.20 s | 2103 MiB | 146 MiB |
| elm-3d-scene 0.19.3 `--optimize` warm | 204.4 ms | 89.0 ms | 2.30× | 200.0 ms | 120.0 ms | 285 MiB | 58 MiB |

### elm make --docs

As for `elm make`, documenting packages with `--docs`: the core library, three widely used packages, the largest corpus package by modules and one of the largest by bytes, with the same `docs.json` from both. A package's `elm.json` admits a range of releases, and nomi builds it as the newest of them does, so only the newest elm is measured. The best of 3 runs counts for each measure.

| workload | elm | nomi | speedup | elm CPU | nomi CPU | elm memory | nomi memory |
|---|--:|--:|--:|--:|--:|--:|--:|
| elm/core 1.0.5, 0.19.3 cold | 78.4 ms | 22.4 ms | 3.51× | 100.0 ms | 40.0 ms | 115 MiB | 26 MiB |
| elm/core 1.0.5, 0.19.3 warm | 77.3 ms | 18.2 ms | 4.25× | 100.0 ms | 30.0 ms | 107 MiB | 23 MiB |
| mdgriffith/elm-ui 1.1.8, 0.19.3 cold | 275.0 ms | 102.7 ms | 2.68× | 440.0 ms | 240.0 ms | 440 MiB | 67 MiB |
| mdgriffith/elm-ui 1.1.8, 0.19.3 warm | 97.3 ms | 45.7 ms | 2.13× | 100.0 ms | 60.0 ms | 126 MiB | 37 MiB |
| rtfeldman/elm-css 18.0.0, 0.19.3 cold | 310.1 ms | 106.1 ms | 2.92× | 570.0 ms | 290.0 ms | 545 MiB | 77 MiB |
| rtfeldman/elm-css 18.0.0, 0.19.3 warm | 198.7 ms | 77.3 ms | 2.57× | 260.0 ms | 130.0 ms | 283 MiB | 61 MiB |
| dillonkearns/elm-graphql 5.0.12, 0.19.3 cold | 187.5 ms | 71.0 ms | 2.64× | 390.0 ms | 200.0 ms | 341 MiB | 49 MiB |
| dillonkearns/elm-graphql 5.0.12, 0.19.3 warm | 66.7 ms | 20.3 ms | 3.29× | 60.0 ms | 30.0 ms | 77 MiB | 26 MiB |
| finos/morphir-elm 22.0.2, 0.19.3 cold | 42.71 s | 359.6 ms | 118.77× | 46.25 s | 2.29 s | 2201 MiB | 250 MiB |
| finos/morphir-elm 22.0.2, 0.19.3 warm | 256.6 ms | 132.5 ms | 1.94× | 520.0 ms | 400.0 ms | 455 MiB | 133 MiB |
| icidasset/elm-material-icons 11.0.0, 0.19.3 cold | 1.14 s | 467.7 ms | 2.44× | 2.28 s | 1.18 s | 2220 MiB | 576 MiB |
| icidasset/elm-material-icons 11.0.0, 0.19.3 warm | 1.08 s | 431.4 ms | 2.49× | 2.18 s | 1.11 s | 2162 MiB | 561 MiB |

### elm-format

nomi, run as `elm-format`, against the pinned elm-format 0.8.8 (its official build, with its default runtime options), from process start to exit. Each corpus app is validated whole with `--validate`, which reads and formats every module and writes nothing; its largest module is formatted through `--stdin`, as editors do, with the same output from both. The best of 5 runs counts for each measure.

| workload | elm-format | nomi | speedup | elm-format CPU | nomi CPU | elm-format memory | nomi memory |
|---|--:|--:|--:|--:|--:|--:|--:|
| exosphere `--validate`, 218 modules, 1940 KiB | 1.17 s | 48.8 ms | 24.06× | 13.28 s | 440.0 ms | 120 MiB | 42 MiB |
| exosphere `--stdin`, 191 KiB | 154.2 ms | 22.1 ms | 6.96× | 200.0 ms | 10.0 ms | 29 MiB | 14 MiB |
| liascript `--validate`, 189 modules, 1361 KiB | 974.3 ms | 41.4 ms | 23.53× | 11.38 s | 400.0 ms | 126 MiB | 42 MiB |
| liascript `--stdin`, 127 KiB | 103.3 ms | 16.5 ms | 6.25× | 120.0 ms | 0.0 ms | 21 MiB | 12 MiB |
| docspell `--validate`, 583 modules, 2478 KiB | 1.92 s | 88.7 ms | 21.58× | 22.85 s | 710.0 ms | 123 MiB | 41 MiB |
| docspell `--stdin`, 104 KiB | 103.6 ms | 22.0 ms | 4.72× | 130.0 ms | 10.0 ms | 21 MiB | 13 MiB |
| elm-3d-scene `--validate`, 59 modules, 929 KiB | 472.2 ms | 24.7 ms | 19.15× | 4.59 s | 160.0 ms | 111 MiB | 36 MiB |
| elm-3d-scene `--stdin`, 132 KiB | 63.5 ms | 5.0 ms | 12.62× | 50.0 ms | 0.0 ms | 20 MiB | 7 MiB |

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
