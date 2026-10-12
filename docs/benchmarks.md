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

Measured on AMD Ryzen 7 5800H with Radeon Graphics (x86_64-linux, 16 threads, 16 GB), rustc 1.99.0. nomi is at or under upstream in 203 of 204 cells; a cell where it isn't is marked ✗.

### elm make

nomi, run as `elm`, against each pinned elm (its official build, with its default runtime options), building each corpus app's entries from process start to exit, in dev and with `--optimize`. A cold build starts without `elm-stuff` or any built dependency: elm's `artifacts.dat` files are removed from its `ELM_HOME`, and nomi's artifact cache is emptied. A warm build follows it with nothing changed. Both tools run offline over the same package sources and write the same JavaScript. The best of 3 runs counts for each measure.

| workload | elm | nomi | speedup | elm CPU | nomi CPU | elm memory | nomi memory |
|---|--:|--:|--:|--:|--:|--:|--:|
| exosphere 0.19.1 cold | 2.24 s | 1.63 s | 1.37× | 8.88 s | 5.57 s | 2379 MiB | 868 MiB |
| exosphere 0.19.1 warm | 399.1 ms | 141.3 ms | 2.82× | 560.0 ms | 380.0 ms | 534 MiB | 126 MiB |
| exosphere 0.19.1 `--optimize` cold | 2.25 s | 1.63 s | 1.38× | 8.93 s | 5.73 s | 2376 MiB | 865 MiB |
| exosphere 0.19.1 `--optimize` warm | 408.1 ms | 141.5 ms | 2.88× | 550.0 ms | 380.0 ms | 535 MiB | 123 MiB |
| liascript 0.19.1 cold | 1.03 s | 543.2 ms | 1.90× | 3.55 s | 1.82 s | 2136 MiB | 231 MiB |
| liascript 0.19.1 warm | 277.7 ms | 108.9 ms | 2.55× | 440.0 ms | 290.0 ms | 367 MiB | 89 MiB |
| liascript 0.19.1 `--optimize` cold | 1.04 s | 538.9 ms | 1.92× | 3.57 s | 1.88 s | 2133 MiB | 235 MiB |
| liascript 0.19.1 `--optimize` warm | 277.3 ms | 111.1 ms | 2.50× | 440.0 ms | 300.0 ms | 368 MiB | 89 MiB |
| docspell 0.19.1 cold | 1.74 s | 1.28 s | 1.36× | 7.04 s | 5.03 s | 2303 MiB | 918 MiB |
| docspell 0.19.1 warm | 423.8 ms | 171.8 ms | 2.47× | 770.0 ms | 550.0 ms | 579 MiB | 124 MiB |
| docspell 0.19.1 `--optimize` cold | 1.77 s | 1.28 s | 1.38× | 7.21 s | 5.02 s | 2329 MiB | 925 MiB |
| docspell 0.19.1 `--optimize` warm | 444.9 ms | 173.9 ms | 2.56× | 790.0 ms | 540.0 ms | 581 MiB | 122 MiB |
| elm-3d-scene 0.19.1 cold | 1.03 s | 306.1 ms | 3.35× | 2.97 s | 1.17 s | 2105 MiB | 155 MiB |
| elm-3d-scene 0.19.1 warm | 189.3 ms | 92.6 ms | 2.04× | 180.0 ms | 120.0 ms | 257 MiB | 65 MiB |
| elm-3d-scene 0.19.1 `--optimize` cold | 1.03 s | 305.1 ms | 3.37× | 2.94 s | 1.18 s | 2107 MiB | 157 MiB |
| elm-3d-scene 0.19.1 `--optimize` warm | 188.2 ms | 92.2 ms | 2.04× | 180.0 ms | 120.0 ms | 257 MiB | 65 MiB |
| exosphere 0.19.2 cold | 2.64 s | 1.63 s | 1.62× | 8.34 s | 5.46 s | 2685 MiB | 873 MiB |
| exosphere 0.19.2 warm | 378.8 ms | 142.3 ms | 2.66× | 530.0 ms | 390.0 ms | 518 MiB | 127 MiB |
| exosphere 0.19.2 `--optimize` cold | 2.62 s | 1.61 s | 1.62× | 8.29 s | 5.58 s | 2680 MiB | 873 MiB |
| exosphere 0.19.2 `--optimize` warm | 380.9 ms | 141.7 ms | 2.69× | 530.0 ms | 390.0 ms | 519 MiB | 125 MiB |
| liascript 0.19.2 cold | 975.8 ms | 532.7 ms | 1.83× | 3.21 s | 1.86 s | 2141 MiB | 235 MiB |
| liascript 0.19.2 warm | 263.0 ms | 109.4 ms | 2.40× | 420.0 ms | 290.0 ms | 360 MiB | 89 MiB |
| liascript 0.19.2 `--optimize` cold | 983.8 ms | 534.7 ms | 1.84× | 3.22 s | 1.87 s | 2148 MiB | 235 MiB |
| liascript 0.19.2 `--optimize` warm | 263.6 ms | 113.9 ms | 2.31× | 430.0 ms | 290.0 ms | 360 MiB | 88 MiB |
| docspell 0.19.2 cold | 1.75 s | 1.28 s | 1.37× | 6.26 s | 5.05 s | 2430 MiB | 905 MiB |
| docspell 0.19.2 warm | 402.8 ms | 172.6 ms | 2.33× | 760.0 ms | 560.0 ms | 566 MiB | 124 MiB |
| docspell 0.19.2 `--optimize` cold | 1.77 s | 1.28 s | 1.38× | 6.38 s | 5.04 s | 2429 MiB | 902 MiB |
| docspell 0.19.2 `--optimize` warm | 420.2 ms | 171.7 ms | 2.45× | 770.0 ms | 540.0 ms | 568 MiB | 122 MiB |
| elm-3d-scene 0.19.2 cold | 1.05 s | 307.7 ms | 3.40× | 2.74 s | 1.16 s | 2118 MiB | 159 MiB |
| elm-3d-scene 0.19.2 warm | 178.7 ms | 92.2 ms | 1.94× | 180.0 ms | 130.0 ms | 253 MiB | 65 MiB |
| elm-3d-scene 0.19.2 `--optimize` cold | 1.04 s | 317.7 ms | 3.27× | 2.74 s | 1.17 s | 2120 MiB | 161 MiB |
| elm-3d-scene 0.19.2 `--optimize` warm | 181.9 ms | 92.2 ms | 1.97× | 180.0 ms | 130.0 ms | 254 MiB | 64 MiB |
| exosphere 0.19.3 cold | 1.72 s | 1.67 s | 1.03× | 7.00 s | 5.77 s | 2239 MiB | 869 MiB |
| exosphere 0.19.3 warm | 414.4 ms | 140.4 ms | 2.95× | 610.0 ms | 380.0 ms | 570 MiB | 125 MiB |
| exosphere 0.19.3 `--optimize` cold | 1.69 s | 1.70 s ✗ | 1.00× | 6.99 s | 5.79 s | 2225 MiB | 874 MiB |
| exosphere 0.19.3 `--optimize` warm | 422.4 ms | 142.8 ms | 2.96× | 610.0 ms | 390.0 ms | 570 MiB | 124 MiB |
| liascript 0.19.3 cold | 1.03 s | 542.3 ms | 1.89× | 3.22 s | 1.90 s | 2154 MiB | 234 MiB |
| liascript 0.19.3 warm | 276.4 ms | 109.9 ms | 2.51× | 460.0 ms | 280.0 ms | 392 MiB | 87 MiB |
| liascript 0.19.3 `--optimize` cold | 1.03 s | 550.6 ms | 1.87× | 3.24 s | 1.85 s | 2157 MiB | 235 MiB |
| liascript 0.19.3 `--optimize` warm | 276.6 ms | 111.8 ms | 2.47× | 450.0 ms | 300.0 ms | 392 MiB | 89 MiB |
| docspell 0.19.3 cold | 1.99 s | 1.30 s | 1.53× | 6.34 s | 5.17 s | 2546 MiB | 933 MiB |
| docspell 0.19.3 warm | 430.2 ms | 171.3 ms | 2.51× | 810.0 ms | 550.0 ms | 619 MiB | 126 MiB |
| docspell 0.19.3 `--optimize` cold | 1.98 s | 1.30 s | 1.52× | 6.34 s | 5.18 s | 2540 MiB | 911 MiB |
| docspell 0.19.3 `--optimize` warm | 435.8 ms | 174.5 ms | 2.50× | 770.0 ms | 550.0 ms | 620 MiB | 123 MiB |
| elm-3d-scene 0.19.3 cold | 1.07 s | 305.0 ms | 3.52× | 2.72 s | 1.20 s | 2105 MiB | 157 MiB |
| elm-3d-scene 0.19.3 warm | 203.7 ms | 91.0 ms | 2.24× | 210.0 ms | 130.0 ms | 285 MiB | 64 MiB |
| elm-3d-scene 0.19.3 `--optimize` cold | 1.06 s | 305.5 ms | 3.48× | 2.71 s | 1.21 s | 2105 MiB | 159 MiB |
| elm-3d-scene 0.19.3 `--optimize` warm | 205.1 ms | 92.3 ms | 2.22× | 210.0 ms | 120.0 ms | 285 MiB | 65 MiB |

### elm make --docs

As for `elm make`, documenting packages with `--docs`: the core library, three widely used packages, the largest corpus package by modules and one of the largest by bytes, with the same `docs.json` from both. A package's `elm.json` admits a range of releases, and nomi builds it as the newest of them does, so only the newest elm is measured. The best of 3 runs counts for each measure.

| workload | elm | nomi | speedup | elm CPU | nomi CPU | elm memory | nomi memory |
|---|--:|--:|--:|--:|--:|--:|--:|
| elm/core 1.0.5, 0.19.3 cold | 83.7 ms | 22.5 ms | 3.72× | 110.0 ms | 40.0 ms | 115 MiB | 27 MiB |
| elm/core 1.0.5, 0.19.3 warm | 72.0 ms | 19.3 ms | 3.74× | 100.0 ms | 30.0 ms | 106 MiB | 24 MiB |
| mdgriffith/elm-ui 1.1.8, 0.19.3 cold | 268.8 ms | 111.2 ms | 2.42× | 440.0 ms | 240.0 ms | 440 MiB | 71 MiB |
| mdgriffith/elm-ui 1.1.8, 0.19.3 warm | 91.0 ms | 46.7 ms | 1.95× | 100.0 ms | 70.0 ms | 126 MiB | 41 MiB |
| rtfeldman/elm-css 18.0.0, 0.19.3 cold | 311.2 ms | 109.1 ms | 2.85× | 570.0 ms | 290.0 ms | 545 MiB | 79 MiB |
| rtfeldman/elm-css 18.0.0, 0.19.3 warm | 195.5 ms | 81.5 ms | 2.40× | 260.0 ms | 130.0 ms | 283 MiB | 66 MiB |
| dillonkearns/elm-graphql 5.0.12, 0.19.3 cold | 185.6 ms | 74.2 ms | 2.50× | 380.0 ms | 200.0 ms | 341 MiB | 52 MiB |
| dillonkearns/elm-graphql 5.0.12, 0.19.3 warm | 62.5 ms | 21.4 ms | 2.92× | 60.0 ms | 30.0 ms | 77 MiB | 27 MiB |
| finos/morphir-elm 22.0.2, 0.19.3 cold | 42.53 s | 365.8 ms | 116.28× | 46.00 s | 2.57 s | 2203 MiB | 282 MiB |
| finos/morphir-elm 22.0.2, 0.19.3 warm | 249.4 ms | 145.8 ms | 1.71× | 510.0 ms | 420.0 ms | 454 MiB | 149 MiB |
| icidasset/elm-material-icons 11.0.0, 0.19.3 cold | 1.13 s | 498.0 ms | 2.28× | 2.26 s | 1.26 s | 2219 MiB | 660 MiB |
| icidasset/elm-material-icons 11.0.0, 0.19.3 warm | 1.06 s | 460.9 ms | 2.29× | 2.16 s | 1.18 s | 2164 MiB | 645 MiB |

### elm-format

nomi, run as `elm-format`, against the pinned elm-format 0.8.8 (its official build, with its default runtime options), from process start to exit. Each corpus app is validated whole with `--validate`, which reads and formats every module and writes nothing; its largest module is formatted through `--stdin`, as editors do, with the same output from both. The best of 5 runs counts for each measure.

| workload | elm-format | nomi | speedup | elm-format CPU | nomi CPU | elm-format memory | nomi memory |
|---|--:|--:|--:|--:|--:|--:|--:|
| exosphere `--validate`, 218 modules, 1940 KiB | 1.17 s | 53.2 ms | 22.05× | 13.31 s | 440.0 ms | 124 MiB | 43 MiB |
| exosphere `--stdin`, 191 KiB | 154.6 ms | 23.1 ms | 6.71× | 210.0 ms | 10.0 ms | 29 MiB | 14 MiB |
| liascript `--validate`, 189 modules, 1361 KiB | 963.5 ms | 39.3 ms | 24.51× | 11.20 s | 400.0 ms | 126 MiB | 42 MiB |
| liascript `--stdin`, 127 KiB | 103.5 ms | 16.8 ms | 6.14× | 130.0 ms | 0.0 ms | 21 MiB | 12 MiB |
| docspell `--validate`, 583 modules, 2478 KiB | 1.90 s | 87.8 ms | 21.68× | 22.77 s | 700.0 ms | 120 MiB | 39 MiB |
| docspell `--stdin`, 104 KiB | 103.6 ms | 22.1 ms | 4.69× | 120.0 ms | 10.0 ms | 21 MiB | 13 MiB |
| elm-3d-scene `--validate`, 59 modules, 929 KiB | 462.2 ms | 24.0 ms | 19.26× | 4.53 s | 170.0 ms | 108 MiB | 37 MiB |
| elm-3d-scene `--stdin`, 132 KiB | 63.5 ms | 4.9 ms | 12.95× | 60.0 ms | 0.0 ms | 20 MiB | 7 MiB |

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
