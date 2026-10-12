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

Measured on AMD Ryzen 7 5800H with Radeon Graphics (x86_64-linux, 16 threads, 16 GB), rustc 1.99.0. nomi is at or under upstream in 202 of 204 cells; a cell where it isn't is marked ✗.

### elm make

nomi, run as `elm`, against each pinned elm (its official build, with its default runtime options), building each corpus app's entries from process start to exit, in dev and with `--optimize`. A cold build starts without `elm-stuff` or any built dependency: elm's `artifacts.dat` files are removed from its `ELM_HOME`, and nomi's artifact cache is emptied. A warm build follows it with nothing changed. Both tools run offline over the same package sources and write the same JavaScript. The best of 3 runs counts for each measure.

| workload | elm | nomi | speedup | elm CPU | nomi CPU | elm memory | nomi memory |
|---|--:|--:|--:|--:|--:|--:|--:|
| exosphere 0.19.1 cold | 2.25 s | 1.82 s | 1.24× | 8.93 s | 5.89 s | 2374 MiB | 920 MiB |
| exosphere 0.19.1 warm | 399.1 ms | 141.2 ms | 2.83× | 560.0 ms | 380.0 ms | 534 MiB | 125 MiB |
| exosphere 0.19.1 `--optimize` cold | 2.26 s | 1.81 s | 1.25× | 8.97 s | 5.94 s | 2376 MiB | 918 MiB |
| exosphere 0.19.1 `--optimize` warm | 410.1 ms | 144.1 ms | 2.85× | 560.0 ms | 390.0 ms | 535 MiB | 123 MiB |
| liascript 0.19.1 cold | 1.03 s | 562.7 ms | 1.83× | 3.55 s | 1.91 s | 2137 MiB | 236 MiB |
| liascript 0.19.1 warm | 265.9 ms | 109.6 ms | 2.43× | 440.0 ms | 290.0 ms | 368 MiB | 89 MiB |
| liascript 0.19.1 `--optimize` cold | 1.04 s | 572.0 ms | 1.82× | 3.59 s | 1.86 s | 2132 MiB | 236 MiB |
| liascript 0.19.1 `--optimize` warm | 286.0 ms | 111.7 ms | 2.56× | 440.0 ms | 290.0 ms | 368 MiB | 87 MiB |
| docspell 0.19.1 cold | 1.76 s | 1.37 s | 1.29× | 6.95 s | 5.40 s | 2322 MiB | 970 MiB |
| docspell 0.19.1 warm | 430.5 ms | 171.5 ms | 2.51× | 760.0 ms | 540.0 ms | 579 MiB | 127 MiB |
| docspell 0.19.1 `--optimize` cold | 1.75 s | 1.36 s | 1.29× | 7.08 s | 5.41 s | 2314 MiB | 977 MiB |
| docspell 0.19.1 `--optimize` warm | 449.2 ms | 176.1 ms | 2.55× | 790.0 ms | 560.0 ms | 582 MiB | 121 MiB |
| elm-3d-scene 0.19.1 cold | 1.02 s | 315.6 ms | 3.24× | 2.95 s | 1.21 s | 2104 MiB | 157 MiB |
| elm-3d-scene 0.19.1 warm | 187.9 ms | 94.2 ms | 1.99× | 180.0 ms | 130.0 ms | 257 MiB | 66 MiB |
| elm-3d-scene 0.19.1 `--optimize` cold | 1.02 s | 314.0 ms | 3.24× | 2.96 s | 1.20 s | 2105 MiB | 158 MiB |
| elm-3d-scene 0.19.1 `--optimize` warm | 176.6 ms | 93.2 ms | 1.89× | 180.0 ms | 130.0 ms | 257 MiB | 65 MiB |
| exosphere 0.19.2 cold | 2.67 s | 1.80 s | 1.48× | 8.34 s | 5.85 s | 2687 MiB | 912 MiB |
| exosphere 0.19.2 warm | 375.4 ms | 140.2 ms | 2.68× | 520.0 ms | 380.0 ms | 518 MiB | 125 MiB |
| exosphere 0.19.2 `--optimize` cold | 2.67 s | 1.80 s | 1.48× | 8.40 s | 5.80 s | 2701 MiB | 922 MiB |
| exosphere 0.19.2 `--optimize` warm | 383.2 ms | 142.9 ms | 2.68× | 530.0 ms | 380.0 ms | 518 MiB | 123 MiB |
| liascript 0.19.2 cold | 980.4 ms | 571.1 ms | 1.72× | 3.19 s | 1.96 s | 2144 MiB | 242 MiB |
| liascript 0.19.2 warm | 259.3 ms | 107.8 ms | 2.41× | 430.0 ms | 290.0 ms | 360 MiB | 89 MiB |
| liascript 0.19.2 `--optimize` cold | 977.4 ms | 568.9 ms | 1.72× | 3.20 s | 1.93 s | 2142 MiB | 232 MiB |
| liascript 0.19.2 `--optimize` warm | 265.2 ms | 109.8 ms | 2.42× | 420.0 ms | 290.0 ms | 360 MiB | 89 MiB |
| docspell 0.19.2 cold | 1.77 s | 1.36 s | 1.30× | 6.36 s | 5.39 s | 2438 MiB | 961 MiB |
| docspell 0.19.2 warm | 401.0 ms | 169.0 ms | 2.37× | 750.0 ms | 550.0 ms | 566 MiB | 122 MiB |
| docspell 0.19.2 `--optimize` cold | 1.77 s | 1.37 s | 1.30× | 6.32 s | 5.42 s | 2428 MiB | 961 MiB |
| docspell 0.19.2 `--optimize` warm | 413.3 ms | 174.0 ms | 2.38× | 760.0 ms | 550.0 ms | 568 MiB | 122 MiB |
| elm-3d-scene 0.19.2 cold | 1.05 s | 315.2 ms | 3.33× | 2.76 s | 1.22 s | 2112 MiB | 157 MiB |
| elm-3d-scene 0.19.2 warm | 182.0 ms | 92.1 ms | 1.98× | 180.0 ms | 130.0 ms | 253 MiB | 64 MiB |
| elm-3d-scene 0.19.2 `--optimize` cold | 1.05 s | 316.7 ms | 3.32× | 2.75 s | 1.22 s | 2114 MiB | 154 MiB |
| elm-3d-scene 0.19.2 `--optimize` warm | 174.6 ms | 92.2 ms | 1.89× | 170.0 ms | 130.0 ms | 254 MiB | 64 MiB |
| exosphere 0.19.3 cold | 1.72 s | 1.82 s ✗ | 0.95× | 7.03 s | 6.10 s | 2219 MiB | 924 MiB |
| exosphere 0.19.3 warm | 407.6 ms | 140.8 ms | 2.89× | 610.0 ms | 380.0 ms | 571 MiB | 122 MiB |
| exosphere 0.19.3 `--optimize` cold | 1.74 s | 1.84 s ✗ | 0.95× | 7.17 s | 5.94 s | 2241 MiB | 915 MiB |
| exosphere 0.19.3 `--optimize` warm | 420.7 ms | 142.4 ms | 2.96× | 620.0 ms | 380.0 ms | 571 MiB | 122 MiB |
| liascript 0.19.3 cold | 1.02 s | 578.8 ms | 1.77× | 3.20 s | 1.90 s | 2151 MiB | 228 MiB |
| liascript 0.19.3 warm | 280.7 ms | 109.4 ms | 2.57× | 450.0 ms | 290.0 ms | 392 MiB | 89 MiB |
| liascript 0.19.3 `--optimize` cold | 1.03 s | 574.6 ms | 1.80× | 3.26 s | 1.97 s | 2150 MiB | 234 MiB |
| liascript 0.19.3 `--optimize` warm | 280.8 ms | 111.8 ms | 2.51× | 460.0 ms | 300.0 ms | 392 MiB | 89 MiB |
| docspell 0.19.3 cold | 1.99 s | 1.40 s | 1.43× | 6.33 s | 5.55 s | 2551 MiB | 975 MiB |
| docspell 0.19.3 warm | 424.9 ms | 173.2 ms | 2.45× | 800.0 ms | 550.0 ms | 619 MiB | 124 MiB |
| docspell 0.19.3 `--optimize` cold | 1.99 s | 1.39 s | 1.43× | 6.34 s | 5.48 s | 2543 MiB | 981 MiB |
| docspell 0.19.3 `--optimize` warm | 437.3 ms | 174.1 ms | 2.51× | 810.0 ms | 550.0 ms | 620 MiB | 124 MiB |
| elm-3d-scene 0.19.3 cold | 1.07 s | 308.9 ms | 3.46× | 2.71 s | 1.22 s | 2108 MiB | 164 MiB |
| elm-3d-scene 0.19.3 warm | 199.0 ms | 92.1 ms | 2.16× | 200.0 ms | 130.0 ms | 284 MiB | 64 MiB |
| elm-3d-scene 0.19.3 `--optimize` cold | 1.07 s | 312.3 ms | 3.43× | 2.72 s | 1.21 s | 2111 MiB | 154 MiB |
| elm-3d-scene 0.19.3 `--optimize` warm | 206.1 ms | 92.1 ms | 2.24× | 200.0 ms | 130.0 ms | 285 MiB | 65 MiB |

### elm make --docs

As for `elm make`, documenting packages with `--docs`: the core library, three widely used packages, the largest corpus package by modules and one of the largest by bytes, with the same `docs.json` from both. A package's `elm.json` admits a range of releases, and nomi builds it as the newest of them does, so only the newest elm is measured. The best of 3 runs counts for each measure.

| workload | elm | nomi | speedup | elm CPU | nomi CPU | elm memory | nomi memory |
|---|--:|--:|--:|--:|--:|--:|--:|
| elm/core 1.0.5, 0.19.3 cold | 81.6 ms | 24.5 ms | 3.33× | 100.0 ms | 40.0 ms | 115 MiB | 26 MiB |
| elm/core 1.0.5, 0.19.3 warm | 75.1 ms | 19.3 ms | 3.90× | 90.0 ms | 30.0 ms | 107 MiB | 24 MiB |
| mdgriffith/elm-ui 1.1.8, 0.19.3 cold | 270.6 ms | 115.0 ms | 2.35× | 430.0 ms | 250.0 ms | 439 MiB | 69 MiB |
| mdgriffith/elm-ui 1.1.8, 0.19.3 warm | 96.3 ms | 49.8 ms | 1.93× | 100.0 ms | 70.0 ms | 126 MiB | 43 MiB |
| rtfeldman/elm-css 18.0.0, 0.19.3 cold | 309.3 ms | 108.4 ms | 2.85× | 560.0 ms | 290.0 ms | 545 MiB | 80 MiB |
| rtfeldman/elm-css 18.0.0, 0.19.3 warm | 193.5 ms | 82.9 ms | 2.33× | 250.0 ms | 140.0 ms | 283 MiB | 63 MiB |
| dillonkearns/elm-graphql 5.0.12, 0.19.3 cold | 182.4 ms | 73.4 ms | 2.48× | 370.0 ms | 200.0 ms | 342 MiB | 52 MiB |
| dillonkearns/elm-graphql 5.0.12, 0.19.3 warm | 61.5 ms | 21.4 ms | 2.87× | 60.0 ms | 30.0 ms | 77 MiB | 27 MiB |
| finos/morphir-elm 22.0.2, 0.19.3 cold | 42.69 s | 374.2 ms | 114.07× | 46.15 s | 2.62 s | 2202 MiB | 277 MiB |
| finos/morphir-elm 22.0.2, 0.19.3 warm | 247.7 ms | 145.6 ms | 1.70× | 490.0 ms | 420.0 ms | 455 MiB | 149 MiB |
| icidasset/elm-material-icons 11.0.0, 0.19.3 cold | 1.13 s | 501.2 ms | 2.26× | 2.26 s | 1.27 s | 2219 MiB | 651 MiB |
| icidasset/elm-material-icons 11.0.0, 0.19.3 warm | 1.05 s | 461.8 ms | 2.27× | 2.16 s | 1.18 s | 2163 MiB | 645 MiB |

### elm-format

nomi, run as `elm-format`, against the pinned elm-format 0.8.8 (its official build, with its default runtime options), from process start to exit. Each corpus app is validated whole with `--validate`, which reads and formats every module and writes nothing; its largest module is formatted through `--stdin`, as editors do, with the same output from both. The best of 5 runs counts for each measure.

| workload | elm-format | nomi | speedup | elm-format CPU | nomi CPU | elm-format memory | nomi memory |
|---|--:|--:|--:|--:|--:|--:|--:|
| exosphere `--validate`, 218 modules, 1940 KiB | 1.18 s | 52.9 ms | 22.36× | 13.41 s | 440.0 ms | 122 MiB | 42 MiB |
| exosphere `--stdin`, 191 KiB | 153.8 ms | 22.3 ms | 6.89× | 200.0 ms | 10.0 ms | 30 MiB | 14 MiB |
| liascript `--validate`, 189 modules, 1361 KiB | 973.9 ms | 42.7 ms | 22.83× | 11.27 s | 380.0 ms | 128 MiB | 43 MiB |
| liascript `--stdin`, 127 KiB | 113.3 ms | 15.8 ms | 7.17× | 120.0 ms | 0.0 ms | 22 MiB | 12 MiB |
| docspell `--validate`, 583 modules, 2478 KiB | 1.91 s | 87.5 ms | 21.88× | 22.73 s | 730.0 ms | 122 MiB | 39 MiB |
| docspell `--stdin`, 104 KiB | 113.9 ms | 22.1 ms | 5.14× | 130.0 ms | 10.0 ms | 21 MiB | 13 MiB |
| elm-3d-scene `--validate`, 59 modules, 929 KiB | 461.5 ms | 25.4 ms | 18.15× | 4.62 s | 160.0 ms | 110 MiB | 36 MiB |
| elm-3d-scene `--stdin`, 132 KiB | 63.4 ms | 4.8 ms | 13.11× | 50.0 ms | 0.0 ms | 20 MiB | 7 MiB |

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
