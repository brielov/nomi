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
| exosphere 0.19.1 cold | 2.27 s | 1.90 s | 1.19× | 8.99 s | 5.86 s | 2379 MiB | 951 MiB |
| exosphere 0.19.1 warm | 398.6 ms | 253.3 ms | 1.57× | 550.0 ms | 740.0 ms ✗ | 534 MiB | 231 MiB |
| exosphere 0.19.1 `--optimize` cold | 2.27 s | 1.91 s | 1.19× | 9.01 s | 5.85 s | 2379 MiB | 958 MiB |
| exosphere 0.19.1 `--optimize` warm | 409.2 ms | 255.2 ms | 1.60× | 560.0 ms | 720.0 ms ✗ | 535 MiB | 232 MiB |
| liascript 0.19.1 cold | 1.03 s | 672.4 ms | 1.53× | 3.52 s | 1.77 s | 2138 MiB | 223 MiB |
| liascript 0.19.1 warm | 277.5 ms | 165.2 ms | 1.68× | 440.0 ms | 440.0 ms | 367 MiB | 107 MiB |
| liascript 0.19.1 `--optimize` cold | 1.03 s | 676.9 ms | 1.53× | 3.55 s | 1.75 s | 2139 MiB | 214 MiB |
| liascript 0.19.1 `--optimize` warm | 285.0 ms | 164.9 ms | 1.73× | 450.0 ms | 440.0 ms | 368 MiB | 106 MiB |
| docspell 0.19.1 cold | 1.76 s | 1.47 s | 1.20× | 6.98 s | 5.32 s | 2325 MiB | 1009 MiB |
| docspell 0.19.1 warm | 431.7 ms | 299.1 ms | 1.44× | 770.0 ms | 910.0 ms ✗ | 579 MiB | 169 MiB |
| docspell 0.19.1 `--optimize` cold | 1.74 s | 1.49 s | 1.17× | 7.09 s | 5.37 s | 2311 MiB | 1000 MiB |
| docspell 0.19.1 `--optimize` warm | 447.8 ms | 302.6 ms | 1.48× | 790.0 ms | 910.0 ms ✗ | 582 MiB | 165 MiB |
| elm-3d-scene 0.19.1 cold | 1.03 s | 600.1 ms | 1.71× | 2.95 s | 940.0 ms | 2106 MiB | 123 MiB |
| elm-3d-scene 0.19.1 warm | 189.3 ms | 119.6 ms | 1.58× | 180.0 ms | 160.0 ms | 257 MiB | 79 MiB |
| elm-3d-scene 0.19.1 `--optimize` cold | 1.02 s | 600.2 ms | 1.69× | 2.94 s | 930.0 ms | 2103 MiB | 120 MiB |
| elm-3d-scene 0.19.1 `--optimize` warm | 188.4 ms | 118.5 ms | 1.59× | 180.0 ms | 160.0 ms | 257 MiB | 79 MiB |
| exosphere 0.19.2 cold | 2.68 s | 1.90 s | 1.41× | 8.41 s | 5.77 s | 2708 MiB | 949 MiB |
| exosphere 0.19.2 warm | 379.3 ms | 252.5 ms | 1.50× | 530.0 ms | 720.0 ms ✗ | 518 MiB | 230 MiB |
| exosphere 0.19.2 `--optimize` cold | 2.64 s | 1.90 s | 1.39× | 8.41 s | 5.88 s | 2688 MiB | 962 MiB |
| exosphere 0.19.2 `--optimize` warm | 387.3 ms | 254.9 ms | 1.52× | 540.0 ms | 730.0 ms ✗ | 519 MiB | 231 MiB |
| liascript 0.19.2 cold | 967.5 ms | 673.7 ms | 1.44× | 3.24 s | 1.75 s | 2145 MiB | 229 MiB |
| liascript 0.19.2 warm | 259.9 ms | 163.5 ms | 1.59× | 430.0 ms | 430.0 ms | 360 MiB | 107 MiB |
| liascript 0.19.2 `--optimize` cold | 979.6 ms | 673.6 ms | 1.45× | 3.24 s | 1.76 s | 2147 MiB | 217 MiB |
| liascript 0.19.2 `--optimize` warm | 265.4 ms | 162.8 ms | 1.63× | 430.0 ms | 430.0 ms | 360 MiB | 107 MiB |
| docspell 0.19.2 cold | 1.76 s | 1.48 s | 1.19× | 6.35 s | 5.32 s | 2430 MiB | 997 MiB |
| docspell 0.19.2 warm | 403.8 ms | 299.4 ms | 1.35× | 750.0 ms | 920.0 ms ✗ | 566 MiB | 168 MiB |
| docspell 0.19.2 `--optimize` cold | 1.77 s | 1.50 s | 1.18× | 6.24 s | 5.32 s | 2429 MiB | 998 MiB |
| docspell 0.19.2 `--optimize` warm | 409.6 ms | 302.8 ms | 1.35× | 760.0 ms | 910.0 ms ✗ | 568 MiB | 163 MiB |
| elm-3d-scene 0.19.2 cold | 1.04 s | 604.3 ms | 1.72× | 2.74 s | 940.0 ms | 2115 MiB | 126 MiB |
| elm-3d-scene 0.19.2 warm | 180.9 ms | 121.8 ms | 1.49× | 170.0 ms | 160.0 ms | 254 MiB | 82 MiB |
| elm-3d-scene 0.19.2 `--optimize` cold | 1.05 s | 600.9 ms | 1.75× | 2.74 s | 940.0 ms | 2117 MiB | 122 MiB |
| elm-3d-scene 0.19.2 `--optimize` warm | 178.7 ms | 119.6 ms | 1.49× | 180.0 ms | 160.0 ms | 254 MiB | 79 MiB |
| exosphere 0.19.3 cold | 1.72 s | 1.94 s ✗ | 0.88× | 7.09 s | 6.00 s | 2232 MiB | 953 MiB |
| exosphere 0.19.3 warm | 409.5 ms | 249.7 ms | 1.64× | 600.0 ms | 720.0 ms ✗ | 570 MiB | 225 MiB |
| exosphere 0.19.3 `--optimize` cold | 1.70 s | 1.94 s ✗ | 0.88× | 7.03 s | 6.01 s | 2227 MiB | 944 MiB |
| exosphere 0.19.3 `--optimize` warm | 420.9 ms | 254.4 ms | 1.65× | 610.0 ms | 730.0 ms ✗ | 571 MiB | 234 MiB |
| liascript 0.19.3 cold | 1.03 s | 676.1 ms | 1.52× | 3.22 s | 1.76 s | 2160 MiB | 226 MiB |
| liascript 0.19.3 warm | 271.9 ms | 160.8 ms | 1.69× | 460.0 ms | 430.0 ms | 392 MiB | 104 MiB |
| liascript 0.19.3 `--optimize` cold | 1.02 s | 677.6 ms | 1.51× | 3.21 s | 1.77 s | 2143 MiB | 222 MiB |
| liascript 0.19.3 `--optimize` warm | 279.9 ms | 162.3 ms | 1.72× | 440.0 ms | 440.0 ms | 392 MiB | 105 MiB |
| docspell 0.19.3 cold | 1.98 s | 1.50 s | 1.32× | 6.34 s | 5.47 s | 2544 MiB | 1006 MiB |
| docspell 0.19.3 warm | 417.0 ms | 291.2 ms | 1.43× | 810.0 ms | 910.0 ms ✗ | 619 MiB | 167 MiB |
| docspell 0.19.3 `--optimize` cold | 1.97 s | 1.52 s | 1.29× | 6.35 s | 5.53 s | 2535 MiB | 1010 MiB |
| docspell 0.19.3 `--optimize` warm | 439.6 ms | 293.7 ms | 1.50× | 800.0 ms | 910.0 ms ✗ | 620 MiB | 166 MiB |
| elm-3d-scene 0.19.3 cold | 1.06 s | 609.0 ms | 1.74× | 2.70 s | 950.0 ms | 2103 MiB | 124 MiB |
| elm-3d-scene 0.19.3 warm | 198.7 ms | 118.5 ms | 1.68× | 200.0 ms | 160.0 ms | 284 MiB | 77 MiB |
| elm-3d-scene 0.19.3 `--optimize` cold | 1.06 s | 603.2 ms | 1.75× | 2.68 s | 930.0 ms | 2106 MiB | 123 MiB |
| elm-3d-scene 0.19.3 `--optimize` warm | 200.9 ms | 118.5 ms | 1.69× | 200.0 ms | 160.0 ms | 285 MiB | 78 MiB |

### elm make --docs

As for `elm make`, documenting packages with `--docs`: the core library, three widely used packages, the largest corpus package by modules and one of the largest by bytes, with the same `docs.json` from both. A package's `elm.json` admits a range of releases, and nomi builds it as the newest of them does, so only the newest elm is measured. The best of 3 runs counts for each measure.

| workload | elm | nomi | speedup | elm CPU | nomi CPU | elm memory | nomi memory |
|---|--:|--:|--:|--:|--:|--:|--:|
| elm/core 1.0.5, 0.19.3 cold | 80.4 ms | 24.5 ms | 3.29× | 100.0 ms | 40.0 ms | 115 MiB | 27 MiB |
| elm/core 1.0.5, 0.19.3 warm | 78.4 ms | 24.6 ms | 3.19× | 90.0 ms | 40.0 ms | 107 MiB | 26 MiB |
| mdgriffith/elm-ui 1.1.8, 0.19.3 cold | 268.9 ms | 191.4 ms | 1.40× | 430.0 ms | 220.0 ms | 439 MiB | 61 MiB |
| mdgriffith/elm-ui 1.1.8, 0.19.3 warm | 93.2 ms | 105.6 ms ✗ | 0.88× | 100.0 ms | 120.0 ms ✗ | 126 MiB | 56 MiB |
| rtfeldman/elm-css 18.0.0, 0.19.3 cold | 309.2 ms | 189.9 ms | 1.63× | 550.0 ms | 260.0 ms | 545 MiB | 73 MiB |
| rtfeldman/elm-css 18.0.0, 0.19.3 warm | 193.6 ms | 105.1 ms | 1.84× | 250.0 ms | 170.0 ms | 283 MiB | 70 MiB |
| dillonkearns/elm-graphql 5.0.12, 0.19.3 cold | 183.4 ms | 122.8 ms | 1.49× | 380.0 ms | 170.0 ms | 341 MiB | 42 MiB |
| dillonkearns/elm-graphql 5.0.12, 0.19.3 warm | 64.6 ms | 35.1 ms | 1.84× | 60.0 ms | 40.0 ms | 77 MiB | 34 MiB |
| finos/morphir-elm 22.0.2, 0.19.3 cold | 42.27 s | 525.0 ms | 80.52× | 45.73 s | 1.94 s | 2202 MiB | 237 MiB |
| finos/morphir-elm 22.0.2, 0.19.3 warm | 251.8 ms | 262.7 ms ✗ | 0.96× | 510.0 ms | 790.0 ms ✗ | 455 MiB | 189 MiB |
| icidasset/elm-material-icons 11.0.0, 0.19.3 cold | 1.13 s | 555.9 ms | 2.04× | 2.25 s | 1.35 s | 2218 MiB | 649 MiB |
| icidasset/elm-material-icons 11.0.0, 0.19.3 warm | 1.05 s | 516.6 ms | 2.03× | 2.16 s | 1.30 s | 2159 MiB | 654 MiB |

### elm-format

nomi, run as `elm-format`, against the pinned elm-format 0.8.8 (its official build, with its default runtime options), from process start to exit. Each corpus app is validated whole with `--validate`, which reads and formats every module and writes nothing; its largest module is formatted through `--stdin`, as editors do, with the same output from both. The best of 5 runs counts for each measure.

| workload | elm-format | nomi | speedup | elm-format CPU | nomi CPU | elm-format memory | nomi memory |
|---|--:|--:|--:|--:|--:|--:|--:|
| exosphere `--validate`, 218 modules, 1940 KiB | 1.18 s | 54.4 ms | 21.74× | 13.41 s | 420.0 ms | 121 MiB | 42 MiB |
| exosphere `--stdin`, 191 KiB | 154.3 ms | 23.3 ms | 6.62× | 210.0 ms | 10.0 ms | 28 MiB | 14 MiB |
| liascript `--validate`, 189 modules, 1361 KiB | 973.7 ms | 37.3 ms | 26.08× | 11.31 s | 380.0 ms | 127 MiB | 41 MiB |
| liascript `--stdin`, 127 KiB | 103.4 ms | 15.6 ms | 6.61× | 130.0 ms | 0.0 ms | 22 MiB | 12 MiB |
| docspell `--validate`, 583 modules, 2478 KiB | 1.91 s | 88.4 ms | 21.65× | 23.00 s | 720.0 ms | 121 MiB | 41 MiB |
| docspell `--stdin`, 104 KiB | 112.9 ms | 22.2 ms | 5.07× | 130.0 ms | 10.0 ms | 21 MiB | 13 MiB |
| elm-3d-scene `--validate`, 59 modules, 929 KiB | 461.8 ms | 25.7 ms | 17.99× | 4.60 s | 170.0 ms | 111 MiB | 36 MiB |
| elm-3d-scene `--stdin`, 132 KiB | 63.1 ms | 4.8 ms | 13.05× | 60.0 ms | 0.0 ms | 19 MiB | 7 MiB |

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
