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
| exosphere 0.19.1 cold | 2.31 s | 1.89 s | 1.22× | 9.26 s | 5.94 s | 2377 MiB | 949 MiB |
| exosphere 0.19.1 warm | 417.8 ms | 258.1 ms | 1.62× | 570.0 ms | 730.0 ms ✗ | 534 MiB | 232 MiB |
| exosphere 0.19.1 `--optimize` cold | 2.32 s | 1.91 s | 1.21× | 9.19 s | 6.01 s | 2370 MiB | 959 MiB |
| exosphere 0.19.1 `--optimize` warm | 424.8 ms | 254.8 ms | 1.67× | 580.0 ms | 740.0 ms ✗ | 535 MiB | 231 MiB |
| liascript 0.19.1 cold | 1.07 s | 624.6 ms | 1.71× | 3.68 s | 1.91 s | 2138 MiB | 231 MiB |
| liascript 0.19.1 warm | 281.1 ms | 161.4 ms | 1.74× | 450.0 ms | 430.0 ms | 367 MiB | 105 MiB |
| liascript 0.19.1 `--optimize` cold | 1.08 s | 625.5 ms | 1.72× | 3.73 s | 1.88 s | 2130 MiB | 222 MiB |
| liascript 0.19.1 `--optimize` warm | 288.6 ms | 164.6 ms | 1.75× | 460.0 ms | 430.0 ms | 368 MiB | 108 MiB |
| docspell 0.19.1 cold | 1.83 s | 1.50 s | 1.22× | 7.28 s | 5.49 s | 2329 MiB | 988 MiB |
| docspell 0.19.1 warm | 449.8 ms | 303.3 ms | 1.48× | 810.0 ms | 910.0 ms ✗ | 580 MiB | 169 MiB |
| docspell 0.19.1 `--optimize` cold | 1.83 s | 1.48 s | 1.24× | 7.39 s | 5.52 s | 2324 MiB | 989 MiB |
| docspell 0.19.1 `--optimize` warm | 460.5 ms | 303.0 ms | 1.52× | 800.0 ms | 920.0 ms ✗ | 582 MiB | 165 MiB |
| elm-3d-scene 0.19.1 cold | 1.06 s | 354.6 ms | 3.00× | 3.06 s | 1.22 s | 2102 MiB | 153 MiB |
| elm-3d-scene 0.19.1 warm | 190.8 ms | 123.8 ms | 1.54× | 180.0 ms | 170.0 ms | 257 MiB | 84 MiB |
| elm-3d-scene 0.19.1 `--optimize` cold | 1.07 s | 358.9 ms | 2.97× | 3.06 s | 1.20 s | 2107 MiB | 158 MiB |
| elm-3d-scene 0.19.1 `--optimize` warm | 189.2 ms | 120.7 ms | 1.57× | 180.0 ms | 170.0 ms | 257 MiB | 81 MiB |
| exosphere 0.19.2 cold | 2.73 s | 1.90 s | 1.44× | 8.62 s | 5.98 s | 2695 MiB | 953 MiB |
| exosphere 0.19.2 warm | 394.2 ms | 255.8 ms | 1.54× | 550.0 ms | 720.0 ms ✗ | 518 MiB | 225 MiB |
| exosphere 0.19.2 `--optimize` cold | 2.72 s | 1.87 s | 1.45× | 8.64 s | 6.00 s | 2692 MiB | 951 MiB |
| exosphere 0.19.2 `--optimize` warm | 397.9 ms | 259.9 ms | 1.53× | 560.0 ms | 740.0 ms ✗ | 519 MiB | 233 MiB |
| liascript 0.19.2 cold | 1.02 s | 634.3 ms | 1.60× | 3.34 s | 1.87 s | 2137 MiB | 241 MiB |
| liascript 0.19.2 warm | 269.9 ms | 162.6 ms | 1.66× | 440.0 ms | 440.0 ms | 360 MiB | 105 MiB |
| liascript 0.19.2 `--optimize` cold | 1.00 s | 639.1 ms | 1.57× | 3.36 s | 1.87 s | 2135 MiB | 220 MiB |
| liascript 0.19.2 `--optimize` warm | 273.3 ms | 165.7 ms | 1.65× | 440.0 ms | 440.0 ms | 360 MiB | 107 MiB |
| docspell 0.19.2 cold | 1.81 s | 1.49 s | 1.22× | 6.50 s | 5.48 s | 2430 MiB | 990 MiB |
| docspell 0.19.2 warm | 415.5 ms | 303.6 ms | 1.37× | 760.0 ms | 920.0 ms ✗ | 566 MiB | 168 MiB |
| docspell 0.19.2 `--optimize` cold | 1.82 s | 1.50 s | 1.21× | 6.55 s | 5.50 s | 2421 MiB | 1009 MiB |
| docspell 0.19.2 `--optimize` warm | 430.7 ms | 304.8 ms | 1.41× | 780.0 ms | 910.0 ms ✗ | 568 MiB | 168 MiB |
| elm-3d-scene 0.19.2 cold | 1.09 s | 355.0 ms | 3.06× | 2.81 s | 1.22 s | 2109 MiB | 155 MiB |
| elm-3d-scene 0.19.2 warm | 180.9 ms | 121.7 ms | 1.49× | 180.0 ms | 160.0 ms | 253 MiB | 82 MiB |
| elm-3d-scene 0.19.2 `--optimize` cold | 1.08 s | 361.7 ms | 2.99× | 2.82 s | 1.17 s | 2114 MiB | 158 MiB |
| elm-3d-scene 0.19.2 `--optimize` warm | 182.1 ms | 122.8 ms | 1.48× | 180.0 ms | 170.0 ms | 254 MiB | 78 MiB |
| exosphere 0.19.3 cold | 1.76 s | 1.92 s ✗ | 0.92× | 7.32 s | 6.03 s | 2241 MiB | 951 MiB |
| exosphere 0.19.3 warm | 424.5 ms | 254.4 ms | 1.67× | 630.0 ms | 730.0 ms ✗ | 570 MiB | 234 MiB |
| exosphere 0.19.3 `--optimize` cold | 1.77 s | 1.91 s ✗ | 0.92× | 7.37 s | 6.12 s | 2237 MiB | 952 MiB |
| exosphere 0.19.3 `--optimize` warm | 435.4 ms | 258.8 ms | 1.68× | 630.0 ms | 750.0 ms ✗ | 571 MiB | 233 MiB |
| liascript 0.19.3 cold | 1.06 s | 635.7 ms | 1.67× | 3.33 s | 1.92 s | 2156 MiB | 233 MiB |
| liascript 0.19.3 warm | 286.6 ms | 162.9 ms | 1.76× | 460.0 ms | 430.0 ms | 392 MiB | 105 MiB |
| liascript 0.19.3 `--optimize` cold | 1.07 s | 641.4 ms | 1.66× | 3.37 s | 1.89 s | 2156 MiB | 225 MiB |
| liascript 0.19.3 `--optimize` warm | 293.3 ms | 165.8 ms | 1.77× | 480.0 ms | 440.0 ms | 392 MiB | 106 MiB |
| docspell 0.19.3 cold | 2.02 s | 1.51 s | 1.33× | 6.55 s | 5.57 s | 2542 MiB | 1013 MiB |
| docspell 0.19.3 warm | 441.0 ms | 296.4 ms | 1.49× | 830.0 ms | 910.0 ms ✗ | 618 MiB | 170 MiB |
| docspell 0.19.3 `--optimize` cold | 2.04 s | 1.50 s | 1.35× | 6.57 s | 5.69 s | 2538 MiB | 1006 MiB |
| docspell 0.19.3 `--optimize` warm | 447.7 ms | 297.2 ms | 1.51× | 840.0 ms | 920.0 ms ✗ | 620 MiB | 166 MiB |
| elm-3d-scene 0.19.3 cold | 1.10 s | 352.0 ms | 3.12× | 2.80 s | 1.22 s | 2107 MiB | 156 MiB |
| elm-3d-scene 0.19.3 warm | 209.3 ms | 120.6 ms | 1.74× | 210.0 ms | 160.0 ms | 284 MiB | 79 MiB |
| elm-3d-scene 0.19.3 `--optimize` cold | 1.10 s | 354.9 ms | 3.10× | 2.81 s | 1.21 s | 2103 MiB | 156 MiB |
| elm-3d-scene 0.19.3 `--optimize` warm | 207.7 ms | 120.6 ms | 1.72× | 210.0 ms | 160.0 ms | 284 MiB | 77 MiB |

### elm make --docs

As for `elm make`, documenting packages with `--docs`: the core library, three widely used packages, the largest corpus package by modules and one of the largest by bytes, with the same `docs.json` from both. A package's `elm.json` admits a range of releases, and nomi builds it as the newest of them does, so only the newest elm is measured. The best of 3 runs counts for each measure.

| workload | elm | nomi | speedup | elm CPU | nomi CPU | elm memory | nomi memory |
|---|--:|--:|--:|--:|--:|--:|--:|
| elm/core 1.0.5, 0.19.3 cold | 82.7 ms | 25.7 ms | 3.21× | 100.0 ms | 40.0 ms | 115 MiB | 27 MiB |
| elm/core 1.0.5, 0.19.3 warm | 72.2 ms | 24.5 ms | 2.94× | 100.0 ms | 40.0 ms | 107 MiB | 26 MiB |
| mdgriffith/elm-ui 1.1.8, 0.19.3 cold | 277.3 ms | 153.7 ms | 1.80× | 440.0 ms | 250.0 ms | 440 MiB | 64 MiB |
| mdgriffith/elm-ui 1.1.8, 0.19.3 warm | 95.4 ms | 109.2 ms ✗ | 0.87× | 110.0 ms | 130.0 ms ✗ | 126 MiB | 54 MiB |
| rtfeldman/elm-css 18.0.0, 0.19.3 cold | 317.5 ms | 150.3 ms | 2.11× | 580.0 ms | 290.0 ms | 545 MiB | 79 MiB |
| rtfeldman/elm-css 18.0.0, 0.19.3 warm | 195.6 ms | 106.0 ms | 1.85× | 260.0 ms | 170.0 ms | 283 MiB | 68 MiB |
| dillonkearns/elm-graphql 5.0.12, 0.19.3 cold | 189.5 ms | 82.6 ms | 2.29× | 380.0 ms | 190.0 ms | 341 MiB | 52 MiB |
| dillonkearns/elm-graphql 5.0.12, 0.19.3 warm | 62.5 ms | 36.1 ms | 1.73× | 60.0 ms | 50.0 ms | 77 MiB | 33 MiB |
| finos/morphir-elm 22.0.2, 0.19.3 cold | 42.64 s | 466.3 ms | 91.44× | 46.18 s | 2.23 s | 2202 MiB | 257 MiB |
| finos/morphir-elm 22.0.2, 0.19.3 warm | 257.8 ms | 264.8 ms ✗ | 0.97× | 510.0 ms | 780.0 ms ✗ | 455 MiB | 184 MiB |
| icidasset/elm-material-icons 11.0.0, 0.19.3 cold | 1.18 s | 559.0 ms | 2.11× | 2.34 s | 1.40 s | 2218 MiB | 662 MiB |
| icidasset/elm-material-icons 11.0.0, 0.19.3 warm | 1.09 s | 526.1 ms | 2.08× | 2.22 s | 1.33 s | 2158 MiB | 655 MiB |

### elm-format

nomi, run as `elm-format`, against the pinned elm-format 0.8.8 (its official build, with its default runtime options), from process start to exit. Each corpus app is validated whole with `--validate`, which reads and formats every module and writes nothing; its largest module is formatted through `--stdin`, as editors do, with the same output from both. The best of 5 runs counts for each measure.

| workload | elm-format | nomi | speedup | elm-format CPU | nomi CPU | elm-format memory | nomi memory |
|---|--:|--:|--:|--:|--:|--:|--:|
| exosphere `--validate`, 218 modules, 1940 KiB | 1.17 s | 52.3 ms | 22.42× | 13.35 s | 430.0 ms | 126 MiB | 42 MiB |
| exosphere `--stdin`, 191 KiB | 154.4 ms | 22.3 ms | 6.94× | 210.0 ms | 10.0 ms | 29 MiB | 14 MiB |
| liascript `--validate`, 189 modules, 1361 KiB | 965.9 ms | 39.8 ms | 24.28× | 11.34 s | 370.0 ms | 128 MiB | 43 MiB |
| liascript `--stdin`, 127 KiB | 103.7 ms | 15.7 ms | 6.59× | 120.0 ms | 10.0 ms | 21 MiB | 12 MiB |
| docspell `--validate`, 583 modules, 2478 KiB | 1.91 s | 89.5 ms | 21.39× | 23.03 s | 720.0 ms | 121 MiB | 40 MiB |
| docspell `--stdin`, 104 KiB | 113.4 ms | 23.2 ms | 4.88× | 130.0 ms | 10.0 ms | 21 MiB | 13 MiB |
| elm-3d-scene `--validate`, 59 modules, 929 KiB | 462.7 ms | 25.9 ms | 17.90× | 4.68 s | 170.0 ms | 111 MiB | 36 MiB |
| elm-3d-scene `--stdin`, 132 KiB | 63.6 ms | 4.8 ms | 13.33× | 50.0 ms | 0.0 ms | 20 MiB | 7 MiB |

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
