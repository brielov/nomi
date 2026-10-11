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

Measured on AMD Ryzen 7 5800H with Radeon Graphics (x86_64-linux, 16 threads, 16 GB), rustc 1.99.0. nomi is at or under upstream in 181 of 204 cells; a cell where it isn't is marked ✗.

### elm make

nomi, run as `elm`, against each pinned elm (its official build, with its default runtime options), building each corpus app's entries from process start to exit, in dev and with `--optimize`. A cold build starts without `elm-stuff` or any built dependency: elm's `artifacts.dat` files are removed from its `ELM_HOME`, and nomi's artifact cache is emptied. A warm build follows it with nothing changed. Both tools run offline over the same package sources and write the same JavaScript. The best of 3 runs counts for each measure.

| workload | elm | nomi | speedup | elm CPU | nomi CPU | elm memory | nomi memory |
|---|--:|--:|--:|--:|--:|--:|--:|
| exosphere 0.19.1 cold | 2.25 s | 2.14 s | 1.05× | 8.78 s | 6.83 s | 2371 MiB | 1200 MiB |
| exosphere 0.19.1 warm | 394.2 ms | 254.7 ms | 1.55× | 540.0 ms | 710.0 ms ✗ | 534 MiB | 242 MiB |
| exosphere 0.19.1 `--optimize` cold | 2.24 s | 2.13 s | 1.05× | 8.81 s | 6.90 s | 2379 MiB | 1187 MiB |
| exosphere 0.19.1 `--optimize` warm | 395.3 ms | 255.7 ms | 1.55× | 540.0 ms | 720.0 ms ✗ | 535 MiB | 243 MiB |
| liascript 0.19.1 cold | 1.02 s | 750.9 ms | 1.36× | 3.53 s | 1.91 s | 2127 MiB | 307 MiB |
| liascript 0.19.1 warm | 265.1 ms | 163.3 ms | 1.62× | 430.0 ms | 420.0 ms | 368 MiB | 110 MiB |
| liascript 0.19.1 `--optimize` cold | 1.03 s | 760.3 ms | 1.36× | 3.53 s | 1.91 s | 2134 MiB | 298 MiB |
| liascript 0.19.1 `--optimize` warm | 276.2 ms | 167.0 ms | 1.65× | 440.0 ms | 430.0 ms | 368 MiB | 111 MiB |
| docspell 0.19.1 cold | 1.71 s | 1.77 s ✗ | 0.97× | 6.96 s | 6.27 s | 2310 MiB | 1230 MiB |
| docspell 0.19.1 warm | 423.2 ms | 301.7 ms | 1.40× | 760.0 ms | 900.0 ms ✗ | 579 MiB | 174 MiB |
| docspell 0.19.1 `--optimize` cold | 1.78 s | 1.78 s | 1.00× | 7.22 s | 6.32 s | 2323 MiB | 1250 MiB |
| docspell 0.19.1 `--optimize` warm | 443.7 ms | 305.5 ms | 1.45× | 790.0 ms | 900.0 ms ✗ | 581 MiB | 173 MiB |
| elm-3d-scene 0.19.1 cold | 1.02 s | 623.0 ms | 1.63× | 2.93 s | 980.0 ms | 2106 MiB | 135 MiB |
| elm-3d-scene 0.19.1 warm | 188.1 ms | 122.7 ms | 1.53× | 180.0 ms | 160.0 ms | 257 MiB | 87 MiB |
| elm-3d-scene 0.19.1 `--optimize` cold | 1.02 s | 628.2 ms | 1.62× | 2.93 s | 990.0 ms | 2103 MiB | 132 MiB |
| elm-3d-scene 0.19.1 `--optimize` warm | 187.2 ms | 124.0 ms | 1.51× | 180.0 ms | 170.0 ms | 257 MiB | 88 MiB |
| exosphere 0.19.2 cold | 2.65 s | 2.13 s | 1.25× | 8.38 s | 6.86 s | 2686 MiB | 1199 MiB |
| exosphere 0.19.2 warm | 377.6 ms | 257.0 ms | 1.47× | 530.0 ms | 710.0 ms ✗ | 518 MiB | 242 MiB |
| exosphere 0.19.2 `--optimize` cold | 2.71 s | 2.15 s | 1.26× | 8.39 s | 6.78 s | 2698 MiB | 1182 MiB |
| exosphere 0.19.2 `--optimize` warm | 378.7 ms | 260.1 ms | 1.46× | 530.0 ms | 730.0 ms ✗ | 519 MiB | 245 MiB |
| liascript 0.19.2 cold | 971.4 ms | 763.2 ms | 1.27× | 3.19 s | 1.94 s | 2147 MiB | 293 MiB |
| liascript 0.19.2 warm | 257.7 ms | 164.4 ms | 1.57× | 430.0 ms | 430.0 ms | 360 MiB | 109 MiB |
| liascript 0.19.2 `--optimize` cold | 978.9 ms | 755.3 ms | 1.30× | 3.22 s | 1.91 s | 2145 MiB | 286 MiB |
| liascript 0.19.2 `--optimize` warm | 265.2 ms | 167.7 ms | 1.58× | 430.0 ms | 430.0 ms | 360 MiB | 112 MiB |
| docspell 0.19.2 cold | 1.76 s | 1.78 s ✗ | 0.99× | 6.30 s | 6.27 s | 2429 MiB | 1259 MiB |
| docspell 0.19.2 warm | 395.6 ms | 306.7 ms | 1.29× | 740.0 ms | 910.0 ms ✗ | 566 MiB | 178 MiB |
| docspell 0.19.2 `--optimize` cold | 1.79 s | 1.77 s | 1.01× | 6.33 s | 6.26 s | 2428 MiB | 1233 MiB |
| docspell 0.19.2 `--optimize` warm | 413.8 ms | 303.5 ms | 1.36× | 730.0 ms | 900.0 ms ✗ | 568 MiB | 172 MiB |
| elm-3d-scene 0.19.2 cold | 1.04 s | 621.3 ms | 1.68× | 2.72 s | 970.0 ms | 2110 MiB | 129 MiB |
| elm-3d-scene 0.19.2 warm | 180.8 ms | 119.6 ms | 1.51× | 180.0 ms | 160.0 ms | 253 MiB | 85 MiB |
| elm-3d-scene 0.19.2 `--optimize` cold | 1.04 s | 624.5 ms | 1.67× | 2.72 s | 990.0 ms | 2111 MiB | 127 MiB |
| elm-3d-scene 0.19.2 `--optimize` warm | 182.1 ms | 122.8 ms | 1.48× | 170.0 ms | 160.0 ms | 254 MiB | 89 MiB |
| exosphere 0.19.3 cold | 1.70 s | 2.20 s ✗ | 0.77× | 6.99 s | 7.04 s ✗ | 2237 MiB | 1187 MiB |
| exosphere 0.19.3 warm | 405.7 ms | 253.8 ms | 1.60× | 600.0 ms | 720.0 ms ✗ | 570 MiB | 246 MiB |
| exosphere 0.19.3 `--optimize` cold | 1.74 s | 2.22 s ✗ | 0.78× | 7.08 s | 7.04 s | 2251 MiB | 1185 MiB |
| exosphere 0.19.3 `--optimize` warm | 418.8 ms | 259.4 ms | 1.61× | 610.0 ms | 720.0 ms ✗ | 571 MiB | 245 MiB |
| liascript 0.19.3 cold | 1.02 s | 760.8 ms | 1.33× | 3.19 s | 1.92 s | 2152 MiB | 287 MiB |
| liascript 0.19.3 warm | 271.0 ms | 160.9 ms | 1.68× | 450.0 ms | 420.0 ms | 392 MiB | 112 MiB |
| liascript 0.19.3 `--optimize` cold | 1.02 s | 768.3 ms | 1.33× | 3.19 s | 1.95 s | 2159 MiB | 286 MiB |
| liascript 0.19.3 `--optimize` warm | 275.2 ms | 163.9 ms | 1.68× | 460.0 ms | 410.0 ms | 392 MiB | 110 MiB |
| docspell 0.19.3 cold | 1.97 s | 1.84 s | 1.07× | 6.40 s | 6.48 s ✗ | 2541 MiB | 1297 MiB |
| docspell 0.19.3 warm | 425.0 ms | 294.2 ms | 1.44× | 790.0 ms | 890.0 ms ✗ | 619 MiB | 172 MiB |
| docspell 0.19.3 `--optimize` cold | 2.00 s | 1.85 s | 1.08× | 6.35 s | 6.51 s ✗ | 2547 MiB | 1252 MiB |
| docspell 0.19.3 `--optimize` warm | 431.3 ms | 293.8 ms | 1.47× | 820.0 ms | 880.0 ms ✗ | 620 MiB | 172 MiB |
| elm-3d-scene 0.19.3 cold | 1.06 s | 624.5 ms | 1.70× | 2.69 s | 980.0 ms | 2107 MiB | 130 MiB |
| elm-3d-scene 0.19.3 warm | 202.9 ms | 122.8 ms | 1.65× | 200.0 ms | 160.0 ms | 284 MiB | 85 MiB |
| elm-3d-scene 0.19.3 `--optimize` cold | 1.05 s | 620.2 ms | 1.69× | 2.67 s | 980.0 ms | 2108 MiB | 127 MiB |
| elm-3d-scene 0.19.3 `--optimize` warm | 202.0 ms | 121.7 ms | 1.66× | 200.0 ms | 160.0 ms | 285 MiB | 85 MiB |

### elm make --docs

As for `elm make`, documenting packages with `--docs`: the core library, three widely used packages, the largest corpus package by modules and one of the largest by bytes, with the same `docs.json` from both. A package's `elm.json` admits a range of releases, and nomi builds it as the newest of them does, so only the newest elm is measured. The best of 3 runs counts for each measure.

| workload | elm | nomi | speedup | elm CPU | nomi CPU | elm memory | nomi memory |
|---|--:|--:|--:|--:|--:|--:|--:|
| elm/core 1.0.5, 0.19.3 cold | 78.4 ms | 25.6 ms | 3.06× | 110.0 ms | 40.0 ms | 115 MiB | 27 MiB |
| elm/core 1.0.5, 0.19.3 warm | 77.4 ms | 25.7 ms | 3.01× | 90.0 ms | 40.0 ms | 107 MiB | 27 MiB |
| mdgriffith/elm-ui 1.1.8, 0.19.3 cold | 264.9 ms | 202.3 ms | 1.31× | 430.0 ms | 230.0 ms | 440 MiB | 68 MiB |
| mdgriffith/elm-ui 1.1.8, 0.19.3 warm | 99.5 ms | 115.5 ms ✗ | 0.86× | 100.0 ms | 130.0 ms ✗ | 126 MiB | 68 MiB |
| rtfeldman/elm-css 18.0.0, 0.19.3 cold | 308.0 ms | 198.9 ms | 1.55× | 570.0 ms | 280.0 ms | 545 MiB | 77 MiB |
| rtfeldman/elm-css 18.0.0, 0.19.3 warm | 191.7 ms | 110.2 ms | 1.74× | 250.0 ms | 180.0 ms | 283 MiB | 75 MiB |
| dillonkearns/elm-graphql 5.0.12, 0.19.3 cold | 178.3 ms | 125.8 ms | 1.42× | 370.0 ms | 170.0 ms | 342 MiB | 44 MiB |
| dillonkearns/elm-graphql 5.0.12, 0.19.3 warm | 64.6 ms | 38.2 ms | 1.69× | 60.0 ms | 50.0 ms | 77 MiB | 34 MiB |
| finos/morphir-elm 22.0.2, 0.19.3 cold | 42.51 s | 573.3 ms | 74.14× | 45.90 s | 2.07 s | 2202 MiB | 271 MiB |
| finos/morphir-elm 22.0.2, 0.19.3 warm | 250.0 ms | 287.7 ms ✗ | 0.87× | 500.0 ms | 820.0 ms ✗ | 455 MiB | 213 MiB |
| icidasset/elm-material-icons 11.0.0, 0.19.3 cold | 1.13 s | 617.6 ms | 1.82× | 2.25 s | 1.49 s | 2216 MiB | 790 MiB |
| icidasset/elm-material-icons 11.0.0, 0.19.3 warm | 1.05 s | 564.6 ms | 1.86× | 2.15 s | 1.43 s | 2163 MiB | 794 MiB |

### elm-format

nomi, run as `elm-format`, against the pinned elm-format 0.8.8 (its official build, with its default runtime options), from process start to exit. Each corpus app is validated whole with `--validate`, which reads and formats every module and writes nothing; its largest module is formatted through `--stdin`, as editors do, with the same output from both. The best of 5 runs counts for each measure.

| workload | elm-format | nomi | speedup | elm-format CPU | nomi CPU | elm-format memory | nomi memory |
|---|--:|--:|--:|--:|--:|--:|--:|
| exosphere `--validate`, 218 modules, 1940 KiB | 1.18 s | 50.4 ms | 23.50× | 13.44 s | 440.0 ms | 123 MiB | 43 MiB |
| exosphere `--stdin`, 191 KiB | 154.8 ms | 22.2 ms | 6.96× | 210.0 ms | 10.0 ms | 29 MiB | 14 MiB |
| liascript `--validate`, 189 modules, 1361 KiB | 974.1 ms | 38.6 ms | 25.26× | 11.43 s | 390.0 ms | 127 MiB | 43 MiB |
| liascript `--stdin`, 127 KiB | 113.8 ms | 16.8 ms | 6.76× | 120.0 ms | 10.0 ms | 21 MiB | 12 MiB |
| docspell `--validate`, 583 modules, 2478 KiB | 1.91 s | 87.8 ms | 21.82× | 23.08 s | 720.0 ms | 121 MiB | 40 MiB |
| docspell `--stdin`, 104 KiB | 113.0 ms | 22.1 ms | 5.10× | 130.0 ms | 10.0 ms | 21 MiB | 13 MiB |
| elm-3d-scene `--validate`, 59 modules, 929 KiB | 463.5 ms | 25.8 ms | 17.98× | 4.58 s | 170.0 ms | 110 MiB | 36 MiB |
| elm-3d-scene `--stdin`, 132 KiB | 63.6 ms | 4.9 ms | 12.84× | 50.0 ms | 0.0 ms | 19 MiB | 7 MiB |

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
