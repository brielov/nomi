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

Measured on AMD Ryzen 7 5800H with Radeon Graphics (x86_64-linux, 16 threads, 16 GB), rustc 1.99.0. nomi is at or under upstream in 162 of 204 cells; a cell where it isn't is marked ✗.

### elm make

nomi, run as `elm`, against each pinned elm (its official build, with its default runtime options), building each corpus app's entries from process start to exit, in dev and with `--optimize`. A cold build starts without `elm-stuff` or any built dependency: elm's `artifacts.dat` files are removed from its `ELM_HOME`, and nomi's artifact cache is emptied. A warm build follows it with nothing changed. Both tools run offline over the same package sources and write the same JavaScript. The best of 3 runs counts for each measure.

| workload | elm | nomi | speedup | elm CPU | nomi CPU | elm memory | nomi memory |
|---|--:|--:|--:|--:|--:|--:|--:|
| exosphere 0.19.1 cold | 2.28 s | 2.90 s ✗ | 0.79× | 8.98 s | 8.01 s | 2376 MiB | 1602 MiB |
| exosphere 0.19.1 warm | 389.9 ms | 275.7 ms | 1.41× | 550.0 ms | 730.0 ms ✗ | 534 MiB | 188 MiB |
| exosphere 0.19.1 `--optimize` cold | 2.27 s | 2.91 s ✗ | 0.78× | 8.98 s | 8.08 s | 2377 MiB | 1589 MiB |
| exosphere 0.19.1 `--optimize` warm | 397.5 ms | 278.5 ms | 1.43× | 560.0 ms | 720.0 ms ✗ | 535 MiB | 190 MiB |
| liascript 0.19.1 cold | 1.03 s | 940.6 ms | 1.10× | 3.55 s | 2.29 s | 2124 MiB | 274 MiB |
| liascript 0.19.1 warm | 265.7 ms | 189.3 ms | 1.40× | 440.0 ms | 460.0 ms ✗ | 368 MiB | 96 MiB |
| liascript 0.19.1 `--optimize` cold | 1.04 s | 940.7 ms | 1.10× | 3.58 s | 2.26 s | 2139 MiB | 277 MiB |
| liascript 0.19.1 `--optimize` warm | 274.6 ms | 189.4 ms | 1.45× | 440.0 ms | 470.0 ms ✗ | 368 MiB | 95 MiB |
| docspell 0.19.1 cold | 1.75 s | 2.42 s ✗ | 0.72× | 7.17 s | 7.54 s ✗ | 2321 MiB | 1555 MiB |
| docspell 0.19.1 warm | 421.5 ms | 347.2 ms | 1.21× | 760.0 ms | 980.0 ms ✗ | 579 MiB | 153 MiB |
| docspell 0.19.1 `--optimize` cold | 1.77 s | 2.44 s ✗ | 0.72× | 7.24 s | 7.64 s ✗ | 2323 MiB | 1570 MiB |
| docspell 0.19.1 `--optimize` warm | 443.0 ms | 348.7 ms | 1.27× | 770.0 ms | 960.0 ms ✗ | 581 MiB | 152 MiB |
| elm-3d-scene 0.19.1 cold | 1.04 s | 770.2 ms | 1.35× | 2.97 s | 1.20 s | 2104 MiB | 128 MiB |
| elm-3d-scene 0.19.1 warm | 188.2 ms | 140.9 ms | 1.34× | 180.0 ms | 180.0 ms | 257 MiB | 73 MiB |
| elm-3d-scene 0.19.1 `--optimize` cold | 1.03 s | 761.1 ms | 1.35× | 2.97 s | 1.18 s | 2102 MiB | 118 MiB |
| elm-3d-scene 0.19.1 `--optimize` warm | 179.7 ms | 139.6 ms | 1.29× | 180.0 ms | 190.0 ms ✗ | 257 MiB | 74 MiB |
| exosphere 0.19.2 cold | 2.53 s | 2.97 s ✗ | 0.85× | 8.42 s | 8.16 s | 2579 MiB | 1605 MiB |
| exosphere 0.19.2 warm | 384.1 ms | 277.4 ms | 1.38× | 550.0 ms | 730.0 ms ✗ | 519 MiB | 186 MiB |
| exosphere 0.19.2 `--optimize` cold | 2.68 s | 2.97 s ✗ | 0.90× | 8.51 s | 8.06 s | 2693 MiB | 1634 MiB |
| exosphere 0.19.2 `--optimize` warm | 393.4 ms | 287.6 ms | 1.37× | 540.0 ms | 740.0 ms ✗ | 519 MiB | 189 MiB |
| liascript 0.19.2 cold | 992.7 ms | 953.0 ms | 1.04× | 3.29 s | 2.31 s | 2142 MiB | 278 MiB |
| liascript 0.19.2 warm | 265.5 ms | 192.5 ms | 1.38× | 440.0 ms | 470.0 ms ✗ | 360 MiB | 96 MiB |
| liascript 0.19.2 `--optimize` cold | 1.00 s | 949.3 ms | 1.06× | 3.31 s | 2.29 s | 2143 MiB | 282 MiB |
| liascript 0.19.2 `--optimize` warm | 270.1 ms | 192.2 ms | 1.41× | 450.0 ms | 470.0 ms ✗ | 360 MiB | 96 MiB |
| docspell 0.19.2 cold | 1.78 s | 2.42 s ✗ | 0.73× | 6.42 s | 7.52 s ✗ | 2423 MiB | 1546 MiB |
| docspell 0.19.2 warm | 413.2 ms | 346.5 ms | 1.19× | 770.0 ms | 980.0 ms ✗ | 566 MiB | 153 MiB |
| docspell 0.19.2 `--optimize` cold | 1.78 s | 2.36 s ✗ | 0.75× | 6.44 s | 7.52 s ✗ | 2433 MiB | 1523 MiB |
| docspell 0.19.2 `--optimize` warm | 416.9 ms | 346.4 ms | 1.20× | 780.0 ms | 980.0 ms ✗ | 568 MiB | 152 MiB |
| elm-3d-scene 0.19.2 cold | 1.07 s | 763.5 ms | 1.40× | 2.80 s | 1.19 s | 2119 MiB | 123 MiB |
| elm-3d-scene 0.19.2 warm | 180.8 ms | 138.5 ms | 1.31× | 180.0 ms | 180.0 ms | 253 MiB | 73 MiB |
| elm-3d-scene 0.19.2 `--optimize` cold | 1.06 s | 771.4 ms | 1.38× | 2.77 s | 1.19 s | 2115 MiB | 121 MiB |
| elm-3d-scene 0.19.2 `--optimize` warm | 181.1 ms | 138.6 ms | 1.31× | 170.0 ms | 180.0 ms ✗ | 254 MiB | 74 MiB |
| exosphere 0.19.3 cold | 1.72 s | 2.90 s ✗ | 0.59× | 7.12 s | 8.14 s ✗ | 2245 MiB | 1539 MiB |
| exosphere 0.19.3 warm | 417.6 ms | 275.4 ms | 1.52× | 600.0 ms | 730.0 ms ✗ | 570 MiB | 187 MiB |
| exosphere 0.19.3 `--optimize` cold | 1.73 s | 2.89 s ✗ | 0.60× | 7.19 s | 8.16 s ✗ | 2233 MiB | 1523 MiB |
| exosphere 0.19.3 `--optimize` warm | 417.9 ms | 282.2 ms | 1.48× | 630.0 ms | 730.0 ms ✗ | 571 MiB | 185 MiB |
| liascript 0.19.3 cold | 1.03 s | 956.0 ms | 1.08× | 3.27 s | 2.31 s | 2155 MiB | 287 MiB |
| liascript 0.19.3 warm | 281.9 ms | 188.0 ms | 1.50× | 460.0 ms | 460.0 ms | 392 MiB | 96 MiB |
| liascript 0.19.3 `--optimize` cold | 1.04 s | 951.1 ms | 1.09× | 3.28 s | 2.29 s | 2157 MiB | 281 MiB |
| liascript 0.19.3 `--optimize` warm | 285.1 ms | 189.3 ms | 1.51× | 470.0 ms | 470.0 ms | 392 MiB | 94 MiB |
| docspell 0.19.3 cold | 1.96 s | 2.47 s ✗ | 0.79× | 6.43 s | 7.71 s ✗ | 2532 MiB | 1551 MiB |
| docspell 0.19.3 warm | 428.0 ms | 338.7 ms | 1.26× | 810.0 ms | 960.0 ms ✗ | 619 MiB | 153 MiB |
| docspell 0.19.3 `--optimize` cold | 2.00 s | 2.47 s ✗ | 0.81× | 6.42 s | 7.81 s ✗ | 2542 MiB | 1582 MiB |
| docspell 0.19.3 `--optimize` warm | 438.2 ms | 340.4 ms | 1.29× | 820.0 ms | 970.0 ms ✗ | 620 MiB | 153 MiB |
| elm-3d-scene 0.19.3 cold | 1.08 s | 761.2 ms | 1.42× | 2.75 s | 1.18 s | 2100 MiB | 117 MiB |
| elm-3d-scene 0.19.3 warm | 204.1 ms | 138.5 ms | 1.47× | 200.0 ms | 190.0 ms | 284 MiB | 74 MiB |
| elm-3d-scene 0.19.3 `--optimize` cold | 1.07 s | 767.5 ms | 1.39× | 2.72 s | 1.19 s | 2107 MiB | 125 MiB |
| elm-3d-scene 0.19.3 `--optimize` warm | 207.1 ms | 140.8 ms | 1.47× | 200.0 ms | 180.0 ms | 285 MiB | 74 MiB |

### elm make --docs

As for `elm make`, documenting packages with `--docs`: the core library, three widely used packages, the largest corpus package by modules and one of the largest by bytes, with the same `docs.json` from both. A package's `elm.json` admits a range of releases, and nomi builds it as the newest of them does, so only the newest elm is measured. The best of 3 runs counts for each measure.

| workload | elm | nomi | speedup | elm CPU | nomi CPU | elm memory | nomi memory |
|---|--:|--:|--:|--:|--:|--:|--:|
| elm/core 1.0.5, 0.19.3 cold | 79.5 ms | 27.6 ms | 2.87× | 110.0 ms | 50.0 ms | 115 MiB | 23 MiB |
| elm/core 1.0.5, 0.19.3 warm | 77.4 ms | 28.7 ms | 2.69× | 90.0 ms | 50.0 ms | 107 MiB | 23 MiB |
| mdgriffith/elm-ui 1.1.8, 0.19.3 cold | 273.9 ms | 255.3 ms | 1.07× | 450.0 ms | 290.0 ms | 440 MiB | 58 MiB |
| mdgriffith/elm-ui 1.1.8, 0.19.3 warm | 98.5 ms | 145.0 ms ✗ | 0.68× | 110.0 ms | 170.0 ms ✗ | 126 MiB | 52 MiB |
| rtfeldman/elm-css 18.0.0, 0.19.3 cold | 311.0 ms | 243.0 ms | 1.28× | 560.0 ms | 340.0 ms | 545 MiB | 63 MiB |
| rtfeldman/elm-css 18.0.0, 0.19.3 warm | 193.0 ms | 136.5 ms | 1.41× | 260.0 ms | 220.0 ms | 283 MiB | 61 MiB |
| dillonkearns/elm-graphql 5.0.12, 0.19.3 cold | 184.3 ms | 155.5 ms | 1.18× | 390.0 ms | 200.0 ms | 341 MiB | 36 MiB |
| dillonkearns/elm-graphql 5.0.12, 0.19.3 warm | 64.7 ms | 41.4 ms | 1.56× | 60.0 ms | 60.0 ms | 77 MiB | 28 MiB |
| finos/morphir-elm 22.0.2, 0.19.3 cold | 42.15 s | 690.9 ms | 61.01× | 45.56 s | 2.45 s | 2201 MiB | 270 MiB |
| finos/morphir-elm 22.0.2, 0.19.3 warm | 246.8 ms | 364.7 ms ✗ | 0.68× | 530.0 ms | 1.06 s ✗ | 455 MiB | 213 MiB |
| icidasset/elm-material-icons 11.0.0, 0.19.3 cold | 1.14 s | 718.6 ms | 1.59× | 2.28 s | 1.78 s | 2216 MiB | 655 MiB |
| icidasset/elm-material-icons 11.0.0, 0.19.3 warm | 1.06 s | 665.8 ms | 1.59× | 2.17 s | 1.73 s | 2156 MiB | 658 MiB |

### elm-format

nomi, run as `elm-format`, against the pinned elm-format 0.8.8 (its official build, with its default runtime options), from process start to exit. Each corpus app is validated whole with `--validate`, which reads and formats every module and writes nothing; its largest module is formatted through `--stdin`, as editors do, with the same output from both. The best of 5 runs counts for each measure.

| workload | elm-format | nomi | speedup | elm-format CPU | nomi CPU | elm-format memory | nomi memory |
|---|--:|--:|--:|--:|--:|--:|--:|
| exosphere `--validate`, 218 modules, 1940 KiB | 1.17 s | 60.8 ms | 19.28× | 13.35 s | 490.0 ms | 121 MiB | 42 MiB |
| exosphere `--stdin`, 191 KiB | 153.8 ms | 24.1 ms | 6.38× | 210.0 ms | 10.0 ms | 29 MiB | 12 MiB |
| liascript `--validate`, 189 modules, 1361 KiB | 974.4 ms | 42.2 ms | 23.11× | 11.32 s | 470.0 ms | 125 MiB | 39 MiB |
| liascript `--stdin`, 127 KiB | 113.1 ms | 16.6 ms | 6.81× | 120.0 ms | 0.0 ms | 21 MiB | 11 MiB |
| docspell `--validate`, 583 modules, 2478 KiB | 1.90 s | 99.1 ms | 19.21× | 22.80 s | 840.0 ms | 123 MiB | 36 MiB |
| docspell `--stdin`, 104 KiB | 113.0 ms | 24.0 ms | 4.71× | 130.0 ms | 10.0 ms | 21 MiB | 12 MiB |
| elm-3d-scene `--validate`, 59 modules, 929 KiB | 461.3 ms | 27.3 ms | 16.92× | 4.67 s | 210.0 ms | 110 MiB | 33 MiB |
| elm-3d-scene `--stdin`, 132 KiB | 63.7 ms | 4.6 ms | 13.73× | 60.0 ms | 0.0 ms | 19 MiB | 7 MiB |

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
