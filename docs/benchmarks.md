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

Measured on AMD Ryzen 9 5900X 12-Core Processor (x86_64-linux, 24 threads, 67 GB), rustc 1.99.0. nomi is at or under upstream in 1155 of 1155 cells; a cell where it isn't is marked ✗.

### elm make

nomi, run as `elm`, against each pinned elm (its official build, with its default runtime options), building each corpus app's entries from process start to exit, in dev and with `--optimize`. A cold build starts without `elm-stuff` or any built dependency: elm's `artifacts.dat` files are removed from its `ELM_HOME`, and nomi's artifact cache is emptied. A warm build follows it with nothing changed. Both tools run offline over the same package sources and write the same JavaScript. The best of 3 runs counts for each measure.

| workload | elm | nomi | speedup | elm CPU | nomi CPU | elm memory | nomi memory |
|---|--:|--:|--:|--:|--:|--:|--:|
| exosphere 0.19.1 cold | 1.68 s | 947.0 ms | 1.78× | 5.91 s | 3.13 s | 3354 MiB | 743 MiB |
| exosphere 0.19.1 warm | 356.5 ms | 88.4 ms | 4.03× | 460.0 ms | 300.0 ms | 549 MiB | 114 MiB |
| exosphere 0.19.1 `--optimize` cold | 1.67 s | 946.4 ms | 1.76× | 5.87 s | 3.08 s | 3359 MiB | 745 MiB |
| exosphere 0.19.1 `--optimize` warm | 366.9 ms | 88.6 ms | 4.14× | 460.0 ms | 290.0 ms | 550 MiB | 112 MiB |
| liascript 0.19.1 cold | 1.07 s | 326.2 ms | 3.29× | 2.91 s | 1.15 s | 2985 MiB | 224 MiB |
| liascript 0.19.1 warm | 242.0 ms | 70.5 ms | 3.43× | 350.0 ms | 230.0 ms | 383 MiB | 82 MiB |
| liascript 0.19.1 `--optimize` cold | 1.09 s | 325.8 ms | 3.33× | 2.91 s | 1.14 s | 2984 MiB | 215 MiB |
| liascript 0.19.1 `--optimize` warm | 251.6 ms | 74.9 ms | 3.36× | 340.0 ms | 240.0 ms | 383 MiB | 83 MiB |
| docspell 0.19.1 cold | 1.31 s | 745.7 ms | 1.75× | 4.87 s | 2.93 s | 3230 MiB | 755 MiB |
| docspell 0.19.1 warm | 380.8 ms | 105.6 ms | 3.61× | 630.0 ms | 420.0 ms | 595 MiB | 119 MiB |
| docspell 0.19.1 `--optimize` cold | 1.35 s | 726.8 ms | 1.85× | 4.80 s | 2.89 s | 3222 MiB | 742 MiB |
| docspell 0.19.1 `--optimize` warm | 399.8 ms | 107.0 ms | 3.74× | 640.0 ms | 410.0 ms | 598 MiB | 116 MiB |
| elm-3d-scene 0.19.1 cold | 1.32 s | 193.2 ms | 6.82× | 2.75 s | 790.0 ms | 3133 MiB | 151 MiB |
| elm-3d-scene 0.19.1 warm | 174.4 ms | 67.8 ms | 2.57× | 170.0 ms | 100.0 ms | 272 MiB | 57 MiB |
| elm-3d-scene 0.19.1 `--optimize` cold | 1.31 s | 191.7 ms | 6.81× | 2.75 s | 780.0 ms | 3131 MiB | 149 MiB |
| elm-3d-scene 0.19.1 `--optimize` warm | 171.3 ms | 67.8 ms | 2.53× | 160.0 ms | 100.0 ms | 272 MiB | 58 MiB |
| elm-spa-example 0.19.1 cold | 228.4 ms | 65.9 ms | 3.47× | 500.0 ms | 220.0 ms | 542 MiB | 61 MiB |
| elm-spa-example 0.19.1 warm | 94.2 ms | 20.3 ms | 4.65× | 90.0 ms | 50.0 ms | 118 MiB | 26 MiB |
| elm-spa-example 0.19.1 `--optimize` cold | 233.5 ms | 65.7 ms | 3.56× | 510.0 ms | 220.0 ms | 542 MiB | 58 MiB |
| elm-spa-example 0.19.1 `--optimize` warm | 96.3 ms | 20.3 ms | 4.75× | 90.0 ms | 50.0 ms | 118 MiB | 26 MiB |
| elm-lang-org 0.19.1 cold | 304.2 ms | 97.5 ms | 3.12× | 640.0 ms | 270.0 ms | 716 MiB | 78 MiB |
| elm-lang-org 0.19.1 warm | 118.5 ms | 23.4 ms | 5.06× | 120.0 ms | 40.0 ms | 158 MiB | 28 MiB |
| elm-lang-org 0.19.1 `--optimize` cold | 306.7 ms | 92.0 ms | 3.33× | 650.0 ms | 260.0 ms | 716 MiB | 78 MiB |
| elm-lang-org 0.19.1 `--optimize` warm | 116.3 ms | 23.4 ms | 4.96× | 110.0 ms | 40.0 ms | 158 MiB | 29 MiB |
| elm-lang-org-editor 0.19.1 cold | 294.0 ms | 90.0 ms | 3.27× | 670.0 ms | 270.0 ms | 739 MiB | 86 MiB |
| elm-lang-org-editor 0.19.1 warm | 104.7 ms | 22.4 ms | 4.68× | 100.0 ms | 50.0 ms | 140 MiB | 29 MiB |
| elm-lang-org-editor 0.19.1 `--optimize` cold | 301.5 ms | 88.9 ms | 3.39× | 670.0 ms | 270.0 ms | 738 MiB | 87 MiB |
| elm-lang-org-editor 0.19.1 `--optimize` warm | 104.7 ms | 22.4 ms | 4.68× | 100.0 ms | 50.0 ms | 140 MiB | 29 MiB |
| todomvc 0.19.1 cold | 167.0 ms | 49.8 ms | 3.35× | 260.0 ms | 110.0 ms | 301 MiB | 41 MiB |
| todomvc 0.19.1 warm | 72.0 ms | 11.8 ms | 6.10× | 60.0 ms | 20.0 ms | 83 MiB | 16 MiB |
| todomvc 0.19.1 `--optimize` cold | 168.1 ms | 45.6 ms | 3.69× | 250.0 ms | 100.0 ms | 302 MiB | 40 MiB |
| todomvc 0.19.1 `--optimize` warm | 72.0 ms | 12.9 ms | 5.59× | 50.0 ms | 10.0 ms | 83 MiB | 16 MiB |
| nixos-search 0.19.1 cold | 406.1 ms | 115.1 ms | 3.53× | 1.12 s | 420.0 ms | 1131 MiB | 115 MiB |
| nixos-search 0.19.1 warm | 128.0 ms | 27.8 ms | 4.61× | 120.0 ms | 60.0 ms | 178 MiB | 38 MiB |
| nixos-search 0.19.1 `--optimize` cold | 394.9 ms | 110.1 ms | 3.59× | 1.11 s | 420.0 ms | 1131 MiB | 109 MiB |
| nixos-search 0.19.1 `--optimize` warm | 129.1 ms | 29.8 ms | 4.33× | 120.0 ms | 60.0 ms | 178 MiB | 39 MiB |
| kite 0.19.1 cold | 493.7 ms | 154.3 ms | 3.20× | 1.38 s | 500.0 ms | 1402 MiB | 130 MiB |
| kite 0.19.1 warm | 170.3 ms | 45.6 ms | 3.74× | 170.0 ms | 80.0 ms | 234 MiB | 48 MiB |
| kite 0.19.1 `--optimize` cold | 488.0 ms | 152.5 ms | 3.20× | 1.39 s | 510.0 ms | 1401 MiB | 130 MiB |
| kite 0.19.1 `--optimize` warm | 170.2 ms | 46.6 ms | 3.65× | 170.0 ms | 80.0 ms | 234 MiB | 49 MiB |
| gingko 0.19.1 cold | 895.3 ms | 232.6 ms | 3.85× | 3.63 s | 1.41 s | 3129 MiB | 208 MiB |
| gingko 0.19.1 warm | 275.4 ms | 74.6 ms | 3.69× | 310.0 ms | 190.0 ms | 415 MiB | 89 MiB |
| gingko 0.19.1 `--optimize` cold | 900.9 ms | 247.8 ms | 3.64× | 3.62 s | 1.41 s | 3128 MiB | 201 MiB |
| gingko 0.19.1 `--optimize` warm | 271.8 ms | 77.3 ms | 3.52× | 310.0 ms | 180.0 ms | 415 MiB | 90 MiB |
| sanderling 0.19.1 cold | 692.6 ms | 295.5 ms | 2.34× | 1.27 s | 600.0 ms | 1484 MiB | 286 MiB |
| sanderling 0.19.1 warm | 119.5 ms | 29.8 ms | 4.01× | 110.0 ms | 60.0 ms | 169 MiB | 38 MiB |
| sanderling 0.19.1 `--optimize` cold | 694.1 ms | 290.5 ms | 2.39× | 1.26 s | 590.0 ms | 1485 MiB | 285 MiB |
| sanderling 0.19.1 `--optimize` warm | 119.4 ms | 30.8 ms | 3.87× | 110.0 ms | 60.0 ms | 169 MiB | 39 MiB |
| elmboy 0.19.1 cold | 644.0 ms | 182.2 ms | 3.54× | 1.28 s | 540.0 ms | 1489 MiB | 186 MiB |
| elmboy 0.19.1 warm | 117.6 ms | 32.9 ms | 3.57× | 130.0 ms | 70.0 ms | 170 MiB | 39 MiB |
| elmboy 0.19.1 `--optimize` cold | 656.3 ms | 182.5 ms | 3.60× | 1.29 s | 540.0 ms | 1490 MiB | 185 MiB |
| elmboy 0.19.1 `--optimize` warm | 119.4 ms | 31.9 ms | 3.74× | 130.0 ms | 70.0 ms | 170 MiB | 38 MiB |
| spades 0.19.1 cold | 276.6 ms | 73.1 ms | 3.78× | 680.0 ms | 280.0 ms | 723 MiB | 77 MiB |
| spades 0.19.1 warm | 104.8 ms | 19.3 ms | 5.44× | 90.0 ms | 30.0 ms | 135 MiB | 27 MiB |
| spades 0.19.1 `--optimize` cold | 274.9 ms | 75.3 ms | 3.65× | 690.0 ms | 290.0 ms | 723 MiB | 73 MiB |
| spades 0.19.1 `--optimize` warm | 105.8 ms | 19.2 ms | 5.51× | 90.0 ms | 30.0 ms | 135 MiB | 27 MiB |
| swarm-dashboard 0.19.1 cold | 196.6 ms | 55.5 ms | 3.54× | 380.0 ms | 170.0 ms | 442 MiB | 55 MiB |
| swarm-dashboard 0.19.1 warm | 81.5 ms | 16.0 ms | 5.09× | 60.0 ms | 20.0 ms | 100 MiB | 20 MiB |
| swarm-dashboard 0.19.1 `--optimize` cold | 203.0 ms | 56.2 ms | 3.61× | 390.0 ms | 170.0 ms | 443 MiB | 55 MiB |
| swarm-dashboard 0.19.1 `--optimize` warm | 82.5 ms | 16.0 ms | 5.15× | 70.0 ms | 20.0 ms | 100 MiB | 20 MiB |
| pomodorolm 0.19.1 cold | 204.0 ms | 59.3 ms | 3.44× | 340.0 ms | 150.0 ms | 407 MiB | 51 MiB |
| pomodorolm 0.19.1 warm | 82.6 ms | 17.0 ms | 4.85× | 70.0 ms | 30.0 ms | 100 MiB | 21 MiB |
| pomodorolm 0.19.1 `--optimize` cold | 210.4 ms | 57.5 ms | 3.66× | 350.0 ms | 150.0 ms | 407 MiB | 53 MiB |
| pomodorolm 0.19.1 `--optimize` warm | 81.5 ms | 17.1 ms | 4.76× | 70.0 ms | 30.0 ms | 101 MiB | 21 MiB |
| bots-tribal-wars 0.19.1 cold | 203.9 ms | 53.1 ms | 3.84× | 360.0 ms | 150.0 ms | 413 MiB | 49 MiB |
| bots-tribal-wars 0.19.1 warm | 71.3 ms | 15.0 ms | 4.76× | 60.0 ms | 20.0 ms | 92 MiB | 20 MiB |
| bots-tribal-wars 0.19.1 `--optimize` cold | 211.3 ms | 54.3 ms | 3.89× | 370.0 ms | 160.0 ms | 413 MiB | 50 MiB |
| bots-tribal-wars 0.19.1 `--optimize` warm | 70.9 ms | 15.0 ms | 4.73× | 60.0 ms | 20.0 ms | 92 MiB | 20 MiB |
| elmstatic-emu 0.19.1 cold | 337.9 ms | 100.5 ms | 3.36× | 800.0 ms | 300.0 ms | 879 MiB | 96 MiB |
| elmstatic-emu 0.19.1 warm | 125.8 ms | 23.4 ms | 5.37× | 100.0 ms | 40.0 ms | 156 MiB | 31 MiB |
| elmstatic-emu 0.19.1 `--optimize` cold | 338.8 ms | 96.2 ms | 3.52× | 800.0 ms | 290.0 ms | 879 MiB | 90 MiB |
| elmstatic-emu 0.19.1 `--optimize` warm | 120.6 ms | 23.4 ms | 5.15× | 100.0 ms | 30.0 ms | 155 MiB | 30 MiB |
| elmstatic-md 0.19.1 cold | 294.7 ms | 83.7 ms | 3.52× | 550.0 ms | 210.0 ms | 628 MiB | 72 MiB |
| elmstatic-md 0.19.1 warm | 96.3 ms | 19.3 ms | 4.99× | 80.0 ms | 20.0 ms | 122 MiB | 25 MiB |
| elmstatic-md 0.19.1 `--optimize` cold | 292.9 ms | 79.3 ms | 3.69× | 540.0 ms | 200.0 ms | 628 MiB | 71 MiB |
| elmstatic-md 0.19.1 `--optimize` warm | 97.3 ms | 21.3 ms | 4.57× | 80.0 ms | 30.0 ms | 122 MiB | 25 MiB |
| emdash 0.19.1 cold | 298.0 ms | 94.9 ms | 3.14× | 650.0 ms | 280.0 ms | 687 MiB | 78 MiB |
| emdash 0.19.1 warm | 116.4 ms | 26.6 ms | 4.37× | 110.0 ms | 50.0 ms | 143 MiB | 32 MiB |
| emdash 0.19.1 `--optimize` cold | 300.0 ms | 96.6 ms | 3.10× | 650.0 ms | 280.0 ms | 687 MiB | 78 MiB |
| emdash 0.19.1 `--optimize` warm | 116.4 ms | 27.7 ms | 4.20× | 110.0 ms | 60.0 ms | 143 MiB | 32 MiB |
| package-elm-lang-org 0.19.1 cold | 233.5 ms | 69.0 ms | 3.38× | 460.0 ms | 190.0 ms | 515 MiB | 58 MiB |
| package-elm-lang-org 0.19.1 warm | 94.1 ms | 20.3 ms | 4.64× | 80.0 ms | 30.0 ms | 112 MiB | 23 MiB |
| package-elm-lang-org 0.19.1 `--optimize` cold | 225.5 ms | 68.1 ms | 3.31× | 460.0 ms | 190.0 ms | 515 MiB | 64 MiB |
| package-elm-lang-org 0.19.1 `--optimize` warm | 93.1 ms | 19.2 ms | 4.84× | 80.0 ms | 30.0 ms | 112 MiB | 23 MiB |
| ui-core-examples 0.19.1 cold | 177.5 ms | 46.6 ms | 3.81× | 240.0 ms | 100.0 ms | 292 MiB | 41 MiB |
| ui-core-examples 0.19.1 warm | 70.9 ms | 10.8 ms | 6.59× | 50.0 ms | 10.0 ms | 81 MiB | 16 MiB |
| ui-core-examples 0.19.1 `--optimize` cold | 168.1 ms | 47.8 ms | 3.51× | 240.0 ms | 100.0 ms | 292 MiB | 41 MiB |
| ui-core-examples 0.19.1 `--optimize` warm | 70.9 ms | 11.8 ms | 6.00× | 50.0 ms | 10.0 ms | 81 MiB | 16 MiB |
| concourse 0.19.1 cold | 826.2 ms | 249.2 ms | 3.32× | 2.54 s | 1.08 s | 2548 MiB | 242 MiB |
| concourse 0.19.1 warm | 246.4 ms | 66.8 ms | 3.69× | 300.0 ms | 190.0 ms | 352 MiB | 76 MiB |
| concourse 0.19.1 `--optimize` cold | 866.0 ms | 260.0 ms | 3.33× | 2.64 s | 1.12 s | 2548 MiB | 245 MiB |
| concourse 0.19.1 `--optimize` warm | 254.9 ms | 70.0 ms | 3.64× | 330.0 ms | 200.0 ms | 351 MiB | 75 MiB |
| guide-repl 0.19.1 cold | 191.4 ms | 60.4 ms | 3.17× | 360.0 ms | 160.0 ms | 400 MiB | 53 MiB |
| guide-repl 0.19.1 warm | 83.7 ms | 16.1 ms | 5.21× | 60.0 ms | 20.0 ms | 96 MiB | 19 MiB |
| guide-repl 0.19.1 `--optimize` cold | 199.9 ms | 56.2 ms | 3.56× | 370.0 ms | 150.0 ms | 400 MiB | 50 MiB |
| guide-repl 0.19.1 `--optimize` warm | 83.6 ms | 16.1 ms | 5.19× | 60.0 ms | 20.0 ms | 96 MiB | 19 MiB |
| exosphere 0.19.2 cold | 2.36 s | 965.8 ms | 2.45× | 6.07 s | 3.29 s | 3741 MiB | 755 MiB |
| exosphere 0.19.2 warm | 354.2 ms | 91.3 ms | 3.88× | 450.0 ms | 310.0 ms | 534 MiB | 114 MiB |
| exosphere 0.19.2 `--optimize` cold | 2.35 s | 953.9 ms | 2.46× | 6.12 s | 3.20 s | 3743 MiB | 744 MiB |
| exosphere 0.19.2 `--optimize` warm | 353.3 ms | 91.0 ms | 3.88× | 450.0 ms | 300.0 ms | 535 MiB | 112 MiB |
| liascript 0.19.2 cold | 958.4 ms | 343.0 ms | 2.79× | 2.58 s | 1.20 s | 2601 MiB | 224 MiB |
| liascript 0.19.2 warm | 239.2 ms | 70.7 ms | 3.38× | 350.0 ms | 240.0 ms | 375 MiB | 82 MiB |
| liascript 0.19.2 `--optimize` cold | 951.9 ms | 392.7 ms | 2.42× | 2.57 s | 1.36 s | 2600 MiB | 212 MiB |
| liascript 0.19.2 `--optimize` warm | 243.1 ms | 75.5 ms | 3.22× | 350.0 ms | 240.0 ms | 376 MiB | 83 MiB |
| docspell 0.19.2 cold | 1.38 s | 858.0 ms | 1.60× | 4.75 s | 3.30 s | 3246 MiB | 737 MiB |
| docspell 0.19.2 warm | 426.6 ms | 122.6 ms | 3.48× | 660.0 ms | 450.0 ms | 582 MiB | 117 MiB |
| docspell 0.19.2 `--optimize` cold | 1.39 s | 885.4 ms | 1.57× | 4.79 s | 3.29 s | 3244 MiB | 752 MiB |
| docspell 0.19.2 `--optimize` warm | 427.3 ms | 123.5 ms | 3.46× | 680.0 ms | 450.0 ms | 584 MiB | 116 MiB |
| elm-3d-scene 0.19.2 cold | 1.26 s | 229.3 ms | 5.49× | 2.67 s | 900.0 ms | 2683 MiB | 152 MiB |
| elm-3d-scene 0.19.2 warm | 182.2 ms | 73.9 ms | 2.47× | 180.0 ms | 110.0 ms | 269 MiB | 59 MiB |
| elm-3d-scene 0.19.2 `--optimize` cold | 1.23 s | 219.5 ms | 5.63× | 2.61 s | 830.0 ms | 2683 MiB | 147 MiB |
| elm-3d-scene 0.19.2 `--optimize` warm | 192.4 ms | 66.8 ms | 2.88× | 190.0 ms | 100.0 ms | 270 MiB | 57 MiB |
| elm-spa-example 0.19.2 cold | 227.0 ms | 76.3 ms | 2.97× | 470.0 ms | 240.0 ms | 479 MiB | 61 MiB |
| elm-spa-example 0.19.2 warm | 92.3 ms | 21.9 ms | 4.21× | 100.0 ms | 50.0 ms | 120 MiB | 26 MiB |
| elm-spa-example 0.19.2 `--optimize` cold | 232.7 ms | 78.1 ms | 2.98× | 510.0 ms | 250.0 ms | 478 MiB | 63 MiB |
| elm-spa-example 0.19.2 `--optimize` warm | 107.7 ms | 21.7 ms | 4.96× | 110.0 ms | 50.0 ms | 119 MiB | 26 MiB |
| elm-lang-org 0.19.2 cold | 304.4 ms | 109.0 ms | 2.79× | 620.0 ms | 290.0 ms | 627 MiB | 77 MiB |
| elm-lang-org 0.19.2 warm | 128.7 ms | 24.9 ms | 5.16× | 120.0 ms | 40.0 ms | 158 MiB | 29 MiB |
| elm-lang-org 0.19.2 `--optimize` cold | 306.0 ms | 104.7 ms | 2.92× | 630.0 ms | 280.0 ms | 627 MiB | 79 MiB |
| elm-lang-org 0.19.2 `--optimize` warm | 121.0 ms | 26.0 ms | 4.65× | 120.0 ms | 40.0 ms | 159 MiB | 28 MiB |
| elm-lang-org-editor 0.19.2 cold | 283.0 ms | 100.6 ms | 2.81× | 630.0 ms | 290.0 ms | 646 MiB | 87 MiB |
| elm-lang-org-editor 0.19.2 warm | 107.7 ms | 23.8 ms | 4.53× | 100.0 ms | 50.0 ms | 141 MiB | 30 MiB |
| elm-lang-org-editor 0.19.2 `--optimize` cold | 291.1 ms | 113.2 ms | 2.57× | 620.0 ms | 320.0 ms | 645 MiB | 88 MiB |
| elm-lang-org-editor 0.19.2 `--optimize` warm | 114.6 ms | 24.8 ms | 4.62× | 110.0 ms | 50.0 ms | 140 MiB | 30 MiB |
| todomvc 0.19.2 cold | 164.2 ms | 53.1 ms | 3.09× | 250.0 ms | 110.0 ms | 268 MiB | 42 MiB |
| todomvc 0.19.2 warm | 71.8 ms | 12.9 ms | 5.55× | 60.0 ms | 10.0 ms | 86 MiB | 16 MiB |
| todomvc 0.19.2 `--optimize` cold | 159.9 ms | 55.5 ms | 2.88× | 240.0 ms | 110.0 ms | 269 MiB | 40 MiB |
| todomvc 0.19.2 `--optimize` warm | 71.8 ms | 13.9 ms | 5.15× | 60.0 ms | 10.0 ms | 86 MiB | 15 MiB |
| nixos-search 0.19.2 cold | 401.4 ms | 130.8 ms | 3.07× | 1.10 s | 470.0 ms | 995 MiB | 109 MiB |
| nixos-search 0.19.2 warm | 143.6 ms | 31.8 ms | 4.51× | 140.0 ms | 70.0 ms | 178 MiB | 38 MiB |
| nixos-search 0.19.2 `--optimize` cold | 398.2 ms | 137.5 ms | 2.90× | 1.07 s | 470.0 ms | 996 MiB | 109 MiB |
| nixos-search 0.19.2 `--optimize` warm | 140.3 ms | 31.5 ms | 4.46× | 140.0 ms | 70.0 ms | 178 MiB | 38 MiB |
| kite 0.19.2 cold | 504.3 ms | 190.6 ms | 2.65× | 1.43 s | 600.0 ms | 1231 MiB | 127 MiB |
| kite 0.19.2 warm | 183.5 ms | 52.8 ms | 3.48× | 190.0 ms | 90.0 ms | 232 MiB | 48 MiB |
| kite 0.19.2 `--optimize` cold | 489.2 ms | 160.4 ms | 3.05× | 1.26 s | 510.0 ms | 1230 MiB | 129 MiB |
| kite 0.19.2 `--optimize` warm | 177.9 ms | 46.7 ms | 3.81× | 190.0 ms | 80.0 ms | 233 MiB | 48 MiB |
| gingko 0.19.2 cold | 902.1 ms | 275.4 ms | 3.28× | 3.38 s | 1.51 s | 2765 MiB | 207 MiB |
| gingko 0.19.2 warm | 288.2 ms | 77.3 ms | 3.73× | 320.0 ms | 190.0 ms | 407 MiB | 88 MiB |
| gingko 0.19.2 `--optimize` cold | 914.3 ms | 284.4 ms | 3.22× | 3.53 s | 1.54 s | 2763 MiB | 204 MiB |
| gingko 0.19.2 `--optimize` warm | 303.4 ms | 90.9 ms | 3.34× | 340.0 ms | 200.0 ms | 406 MiB | 88 MiB |
| sanderling 0.19.2 cold | 650.5 ms | 342.4 ms | 1.90× | 1.21 s | 660.0 ms | 1256 MiB | 284 MiB |
| sanderling 0.19.2 warm | 127.3 ms | 33.7 ms | 3.78× | 120.0 ms | 60.0 ms | 169 MiB | 39 MiB |
| sanderling 0.19.2 `--optimize` cold | 662.6 ms | 337.2 ms | 1.97× | 1.22 s | 670.0 ms | 1256 MiB | 286 MiB |
| sanderling 0.19.2 `--optimize` warm | 124.3 ms | 32.1 ms | 3.87× | 130.0 ms | 70.0 ms | 170 MiB | 39 MiB |
| elmboy 0.19.2 cold | 633.0 ms | 215.1 ms | 2.94× | 1.24 s | 620.0 ms | 1275 MiB | 187 MiB |
| elmboy 0.19.2 warm | 128.8 ms | 37.8 ms | 3.40× | 140.0 ms | 70.0 ms | 169 MiB | 39 MiB |
| elmboy 0.19.2 `--optimize` cold | 641.7 ms | 211.8 ms | 3.03× | 1.23 s | 610.0 ms | 1276 MiB | 187 MiB |
| elmboy 0.19.2 `--optimize` warm | 131.7 ms | 37.3 ms | 3.53× | 130.0 ms | 80.0 ms | 170 MiB | 40 MiB |
| spades 0.19.2 cold | 260.7 ms | 94.3 ms | 2.76× | 670.0 ms | 320.0 ms | 633 MiB | 71 MiB |
| spades 0.19.2 warm | 107.4 ms | 23.0 ms | 4.67× | 100.0 ms | 40.0 ms | 136 MiB | 27 MiB |
| spades 0.19.2 `--optimize` cold | 264.1 ms | 86.0 ms | 3.07× | 670.0 ms | 330.0 ms | 632 MiB | 77 MiB |
| spades 0.19.2 `--optimize` warm | 113.4 ms | 22.1 ms | 5.12× | 100.0 ms | 30.0 ms | 136 MiB | 27 MiB |
| swarm-dashboard 0.19.2 cold | 211.4 ms | 63.9 ms | 3.31× | 390.0 ms | 180.0 ms | 388 MiB | 55 MiB |
| swarm-dashboard 0.19.2 warm | 89.7 ms | 18.4 ms | 4.88× | 70.0 ms | 30.0 ms | 103 MiB | 20 MiB |
| swarm-dashboard 0.19.2 `--optimize` cold | 205.0 ms | 64.7 ms | 3.17× | 380.0 ms | 200.0 ms | 388 MiB | 55 MiB |
| swarm-dashboard 0.19.2 `--optimize` warm | 85.2 ms | 17.7 ms | 4.80× | 70.0 ms | 30.0 ms | 103 MiB | 20 MiB |
| pomodorolm 0.19.2 cold | 208.7 ms | 66.2 ms | 3.15× | 340.0 ms | 170.0 ms | 360 MiB | 54 MiB |
| pomodorolm 0.19.2 warm | 90.4 ms | 19.6 ms | 4.61× | 70.0 ms | 30.0 ms | 103 MiB | 21 MiB |
| pomodorolm 0.19.2 `--optimize` cold | 207.1 ms | 68.6 ms | 3.02× | 330.0 ms | 170.0 ms | 361 MiB | 54 MiB |
| pomodorolm 0.19.2 `--optimize` warm | 85.7 ms | 19.3 ms | 4.44× | 80.0 ms | 30.0 ms | 103 MiB | 21 MiB |
| bots-tribal-wars 0.19.2 cold | 209.1 ms | 65.3 ms | 3.20× | 350.0 ms | 180.0 ms | 363 MiB | 48 MiB |
| bots-tribal-wars 0.19.2 warm | 82.3 ms | 16.2 ms | 5.09× | 60.0 ms | 20.0 ms | 95 MiB | 20 MiB |
| bots-tribal-wars 0.19.2 `--optimize` cold | 210.8 ms | 61.0 ms | 3.46× | 370.0 ms | 170.0 ms | 363 MiB | 47 MiB |
| bots-tribal-wars 0.19.2 `--optimize` warm | 79.9 ms | 16.1 ms | 4.95× | 70.0 ms | 30.0 ms | 95 MiB | 20 MiB |
| elmstatic-emu 0.19.2 cold | 318.1 ms | 107.7 ms | 2.95× | 780.0 ms | 330.0 ms | 762 MiB | 92 MiB |
| elmstatic-emu 0.19.2 warm | 128.3 ms | 26.0 ms | 4.93× | 120.0 ms | 40.0 ms | 155 MiB | 30 MiB |
| elmstatic-emu 0.19.2 `--optimize` cold | 322.7 ms | 101.6 ms | 3.18× | 760.0 ms | 310.0 ms | 762 MiB | 96 MiB |
| elmstatic-emu 0.19.2 `--optimize` warm | 122.9 ms | 23.5 ms | 5.24× | 110.0 ms | 30.0 ms | 155 MiB | 30 MiB |
| elmstatic-md 0.19.2 cold | 259.9 ms | 82.6 ms | 3.15× | 480.0 ms | 200.0 ms | 541 MiB | 74 MiB |
| elmstatic-md 0.19.2 warm | 92.0 ms | 20.3 ms | 4.54× | 80.0 ms | 30.0 ms | 123 MiB | 24 MiB |
| elmstatic-md 0.19.2 `--optimize` cold | 265.2 ms | 83.6 ms | 3.17× | 490.0 ms | 220.0 ms | 541 MiB | 72 MiB |
| elmstatic-md 0.19.2 `--optimize` warm | 92.1 ms | 21.3 ms | 4.32× | 80.0 ms | 30.0 ms | 123 MiB | 24 MiB |
| emdash 0.19.2 cold | 285.9 ms | 94.3 ms | 3.03× | 610.0 ms | 280.0 ms | 607 MiB | 77 MiB |
| emdash 0.19.2 warm | 108.9 ms | 26.7 ms | 4.08× | 110.0 ms | 60.0 ms | 144 MiB | 31 MiB |
| emdash 0.19.2 `--optimize` cold | 285.1 ms | 95.2 ms | 2.99× | 620.0 ms | 290.0 ms | 606 MiB | 76 MiB |
| emdash 0.19.2 `--optimize` warm | 112.2 ms | 27.7 ms | 4.05× | 120.0 ms | 60.0 ms | 145 MiB | 31 MiB |
| package-elm-lang-org 0.19.2 cold | 229.1 ms | 75.0 ms | 3.05× | 420.0 ms | 200.0 ms | 452 MiB | 61 MiB |
| package-elm-lang-org 0.19.2 warm | 88.9 ms | 20.3 ms | 4.37× | 80.0 ms | 40.0 ms | 115 MiB | 24 MiB |
| package-elm-lang-org 0.19.2 `--optimize` cold | 229.3 ms | 84.3 ms | 2.72× | 420.0 ms | 220.0 ms | 452 MiB | 60 MiB |
| package-elm-lang-org 0.19.2 `--optimize` warm | 86.8 ms | 22.7 ms | 3.81× | 80.0 ms | 40.0 ms | 115 MiB | 24 MiB |
| ui-core-examples 0.19.2 cold | 165.7 ms | 53.6 ms | 3.09× | 240.0 ms | 110.0 ms | 260 MiB | 39 MiB |
| ui-core-examples 0.19.2 warm | 72.2 ms | 11.9 ms | 6.09× | 60.0 ms | 10.0 ms | 84 MiB | 15 MiB |
| ui-core-examples 0.19.2 `--optimize` cold | 165.4 ms | 56.3 ms | 2.94× | 230.0 ms | 120.0 ms | 260 MiB | 41 MiB |
| ui-core-examples 0.19.2 `--optimize` warm | 71.4 ms | 11.8 ms | 6.04× | 50.0 ms | 20.0 ms | 84 MiB | 15 MiB |
| concourse 0.19.2 cold | 846.3 ms | 283.7 ms | 2.98× | 2.53 s | 1.22 s | 2201 MiB | 235 MiB |
| concourse 0.19.2 warm | 260.0 ms | 75.6 ms | 3.44× | 320.0 ms | 210.0 ms | 346 MiB | 75 MiB |
| concourse 0.19.2 `--optimize` cold | 826.9 ms | 291.4 ms | 2.84× | 2.52 s | 1.23 s | 2201 MiB | 239 MiB |
| concourse 0.19.2 `--optimize` warm | 269.0 ms | 75.6 ms | 3.56× | 340.0 ms | 220.0 ms | 346 MiB | 74 MiB |
| guide-repl 0.19.2 cold | 184.0 ms | 61.2 ms | 3.01× | 350.0 ms | 170.0 ms | 353 MiB | 51 MiB |
| guide-repl 0.19.2 warm | 84.4 ms | 17.2 ms | 4.92× | 70.0 ms | 20.0 ms | 98 MiB | 19 MiB |
| guide-repl 0.19.2 `--optimize` cold | 186.0 ms | 62.1 ms | 3.00× | 350.0 ms | 170.0 ms | 354 MiB | 53 MiB |
| guide-repl 0.19.2 `--optimize` warm | 86.9 ms | 17.3 ms | 5.02× | 70.0 ms | 20.0 ms | 98 MiB | 19 MiB |
| exosphere 0.19.3 cold | 1.62 s | 1.09 s | 1.49× | 5.35 s | 3.34 s | 3231 MiB | 771 MiB |
| exosphere 0.19.3 warm | 424.7 ms | 90.1 ms | 4.71× | 560.0 ms | 300.0 ms | 586 MiB | 114 MiB |
| exosphere 0.19.3 `--optimize` cold | 1.60 s | 1.10 s | 1.46× | 5.19 s | 3.59 s | 3239 MiB | 746 MiB |
| exosphere 0.19.3 `--optimize` warm | 413.0 ms | 93.6 ms | 4.41× | 530.0 ms | 300.0 ms | 587 MiB | 113 MiB |
| liascript 0.19.3 cold | 1.00 s | 379.8 ms | 2.64× | 2.61 s | 1.25 s | 2482 MiB | 218 MiB |
| liascript 0.19.3 warm | 270.5 ms | 71.7 ms | 3.77× | 390.0 ms | 230.0 ms | 407 MiB | 84 MiB |
| liascript 0.19.3 `--optimize` cold | 1.00 s | 362.0 ms | 2.76× | 2.68 s | 1.21 s | 2482 MiB | 221 MiB |
| liascript 0.19.3 `--optimize` warm | 269.8 ms | 86.3 ms | 3.13× | 380.0 ms | 270.0 ms | 408 MiB | 83 MiB |
| docspell 0.19.3 cold | 1.35 s | 849.9 ms | 1.59× | 4.35 s | 3.12 s | 3250 MiB | 763 MiB |
| docspell 0.19.3 warm | 399.7 ms | 105.6 ms | 3.78× | 690.0 ms | 420.0 ms | 634 MiB | 120 MiB |
| docspell 0.19.3 `--optimize` cold | 1.44 s | 877.6 ms | 1.64× | 4.72 s | 3.20 s | 3245 MiB | 758 MiB |
| docspell 0.19.3 `--optimize` warm | 461.0 ms | 129.4 ms | 3.56× | 720.0 ms | 450.0 ms | 636 MiB | 115 MiB |
| elm-3d-scene 0.19.3 cold | 1.16 s | 200.0 ms | 5.78× | 2.46 s | 810.0 ms | 2608 MiB | 148 MiB |
| elm-3d-scene 0.19.3 warm | 207.9 ms | 68.8 ms | 3.02× | 200.0 ms | 100.0 ms | 300 MiB | 57 MiB |
| elm-3d-scene 0.19.3 `--optimize` cold | 1.14 s | 220.9 ms | 5.16× | 2.39 s | 820.0 ms | 2609 MiB | 147 MiB |
| elm-3d-scene 0.19.3 `--optimize` warm | 192.4 ms | 69.9 ms | 2.75× | 190.0 ms | 110.0 ms | 300 MiB | 58 MiB |
| elm-spa-example 0.19.3 cold | 216.9 ms | 67.5 ms | 3.22× | 460.0 ms | 220.0 ms | 458 MiB | 59 MiB |
| elm-spa-example 0.19.3 warm | 96.3 ms | 20.3 ms | 4.74× | 100.0 ms | 40.0 ms | 126 MiB | 26 MiB |
| elm-spa-example 0.19.3 `--optimize` cold | 223.5 ms | 67.8 ms | 3.30× | 460.0 ms | 220.0 ms | 459 MiB | 60 MiB |
| elm-spa-example 0.19.3 `--optimize` warm | 98.5 ms | 20.3 ms | 4.85× | 100.0 ms | 50.0 ms | 126 MiB | 26 MiB |
| elm-lang-org 0.19.3 cold | 303.7 ms | 114.2 ms | 2.66× | 620.0 ms | 300.0 ms | 599 MiB | 79 MiB |
| elm-lang-org 0.19.3 warm | 136.2 ms | 24.8 ms | 5.48× | 130.0 ms | 40.0 ms | 168 MiB | 29 MiB |
| elm-lang-org 0.19.3 `--optimize` cold | 286.8 ms | 96.4 ms | 2.98× | 590.0 ms | 260.0 ms | 599 MiB | 78 MiB |
| elm-lang-org 0.19.3 `--optimize` warm | 123.0 ms | 22.4 ms | 5.49× | 120.0 ms | 30.0 ms | 168 MiB | 29 MiB |
| elm-lang-org-editor 0.19.3 cold | 281.9 ms | 93.2 ms | 3.02× | 590.0 ms | 280.0 ms | 616 MiB | 89 MiB |
| elm-lang-org-editor 0.19.3 warm | 112.1 ms | 22.4 ms | 5.01× | 100.0 ms | 50.0 ms | 150 MiB | 30 MiB |
| elm-lang-org-editor 0.19.3 `--optimize` cold | 290.6 ms | 97.4 ms | 2.98× | 600.0 ms | 280.0 ms | 616 MiB | 87 MiB |
| elm-lang-org-editor 0.19.3 `--optimize` warm | 109.0 ms | 22.4 ms | 4.86× | 110.0 ms | 50.0 ms | 150 MiB | 30 MiB |
| todomvc 0.19.3 cold | 151.5 ms | 48.7 ms | 3.11× | 220.0 ms | 100.0 ms | 257 MiB | 40 MiB |
| todomvc 0.19.3 warm | 69.9 ms | 11.8 ms | 5.91× | 60.0 ms | 10.0 ms | 90 MiB | 16 MiB |
| todomvc 0.19.3 `--optimize` cold | 160.1 ms | 50.9 ms | 3.15× | 230.0 ms | 110.0 ms | 257 MiB | 41 MiB |
| todomvc 0.19.3 `--optimize` warm | 71.9 ms | 12.9 ms | 5.57× | 60.0 ms | 10.0 ms | 89 MiB | 16 MiB |
| nixos-search 0.19.3 cold | 368.3 ms | 115.4 ms | 3.19× | 1.00 s | 430.0 ms | 951 MiB | 112 MiB |
| nixos-search 0.19.3 warm | 133.3 ms | 27.7 ms | 4.81× | 140.0 ms | 60.0 ms | 191 MiB | 39 MiB |
| nixos-search 0.19.3 `--optimize` cold | 384.3 ms | 121.7 ms | 3.16× | 1.01 s | 420.0 ms | 951 MiB | 109 MiB |
| nixos-search 0.19.3 `--optimize` warm | 136.4 ms | 28.7 ms | 4.76× | 130.0 ms | 60.0 ms | 191 MiB | 38 MiB |
| kite 0.19.3 cold | 463.4 ms | 179.7 ms | 2.58× | 1.27 s | 550.0 ms | 1176 MiB | 129 MiB |
| kite 0.19.3 warm | 202.9 ms | 45.7 ms | 4.44× | 200.0 ms | 80.0 ms | 251 MiB | 48 MiB |
| kite 0.19.3 `--optimize` cold | 486.0 ms | 188.7 ms | 2.58× | 1.29 s | 560.0 ms | 1177 MiB | 130 MiB |
| kite 0.19.3 `--optimize` warm | 209.9 ms | 48.8 ms | 4.30× | 210.0 ms | 80.0 ms | 251 MiB | 49 MiB |
| gingko 0.19.3 cold | 880.2 ms | 266.6 ms | 3.30× | 3.36 s | 1.46 s | 2651 MiB | 209 MiB |
| gingko 0.19.3 warm | 314.5 ms | 76.6 ms | 4.10× | 360.0 ms | 190.0 ms | 443 MiB | 89 MiB |
| gingko 0.19.3 `--optimize` cold | 865.4 ms | 261.1 ms | 3.31× | 3.32 s | 1.46 s | 2650 MiB | 208 MiB |
| gingko 0.19.3 `--optimize` warm | 333.9 ms | 78.4 ms | 4.26× | 370.0 ms | 190.0 ms | 442 MiB | 90 MiB |
| sanderling 0.19.3 cold | 643.0 ms | 348.4 ms | 1.85× | 1.18 s | 690.0 ms | 1179 MiB | 285 MiB |
| sanderling 0.19.3 warm | 138.2 ms | 32.1 ms | 4.31× | 140.0 ms | 60.0 ms | 183 MiB | 39 MiB |
| sanderling 0.19.3 `--optimize` cold | 650.3 ms | 325.9 ms | 2.00× | 1.19 s | 660.0 ms | 1179 MiB | 278 MiB |
| sanderling 0.19.3 `--optimize` warm | 134.0 ms | 33.0 ms | 4.07× | 140.0 ms | 60.0 ms | 184 MiB | 39 MiB |
| elmboy 0.19.3 cold | 547.9 ms | 189.2 ms | 2.90× | 1.10 s | 550.0 ms | 1207 MiB | 185 MiB |
| elmboy 0.19.3 warm | 127.1 ms | 31.9 ms | 3.98× | 130.0 ms | 70.0 ms | 183 MiB | 39 MiB |
| elmboy 0.19.3 `--optimize` cold | 533.7 ms | 189.2 ms | 2.82× | 1.06 s | 540.0 ms | 1207 MiB | 184 MiB |
| elmboy 0.19.3 `--optimize` warm | 131.1 ms | 31.9 ms | 4.11× | 130.0 ms | 70.0 ms | 184 MiB | 39 MiB |
| spades 0.19.3 cold | 217.9 ms | 78.3 ms | 2.78× | 580.0 ms | 280.0 ms | 604 MiB | 74 MiB |
| spades 0.19.3 warm | 110.1 ms | 20.2 ms | 5.44× | 100.0 ms | 30.0 ms | 145 MiB | 27 MiB |
| spades 0.19.3 `--optimize` cold | 222.2 ms | 74.2 ms | 2.99× | 590.0 ms | 290.0 ms | 604 MiB | 75 MiB |
| spades 0.19.3 `--optimize` warm | 110.0 ms | 20.3 ms | 5.42× | 90.0 ms | 40.0 ms | 145 MiB | 26 MiB |
| swarm-dashboard 0.19.3 cold | 178.6 ms | 58.4 ms | 3.06× | 330.0 ms | 180.0 ms | 371 MiB | 55 MiB |
| swarm-dashboard 0.19.3 warm | 84.7 ms | 16.0 ms | 5.28× | 70.0 ms | 20.0 ms | 108 MiB | 21 MiB |
| swarm-dashboard 0.19.3 `--optimize` cold | 171.2 ms | 58.2 ms | 2.94× | 320.0 ms | 170.0 ms | 371 MiB | 54 MiB |
| swarm-dashboard 0.19.3 `--optimize` warm | 81.5 ms | 16.0 ms | 5.08× | 70.0 ms | 20.0 ms | 108 MiB | 21 MiB |
| pomodorolm 0.19.3 cold | 178.6 ms | 59.4 ms | 3.01× | 300.0 ms | 150.0 ms | 346 MiB | 51 MiB |
| pomodorolm 0.19.3 warm | 80.4 ms | 17.1 ms | 4.70× | 70.0 ms | 30.0 ms | 108 MiB | 21 MiB |
| pomodorolm 0.19.3 `--optimize` cold | 181.8 ms | 61.5 ms | 2.96× | 300.0 ms | 160.0 ms | 346 MiB | 55 MiB |
| pomodorolm 0.19.3 `--optimize` warm | 78.3 ms | 17.2 ms | 4.56× | 70.0 ms | 30.0 ms | 108 MiB | 22 MiB |
| bots-tribal-wars 0.19.3 cold | 171.6 ms | 56.2 ms | 3.05× | 300.0 ms | 160.0 ms | 348 MiB | 49 MiB |
| bots-tribal-wars 0.19.3 warm | 74.1 ms | 15.0 ms | 4.94× | 60.0 ms | 20.0 ms | 99 MiB | 19 MiB |
| bots-tribal-wars 0.19.3 `--optimize` cold | 164.9 ms | 56.3 ms | 2.93× | 300.0 ms | 160.0 ms | 348 MiB | 50 MiB |
| bots-tribal-wars 0.19.3 `--optimize` warm | 75.2 ms | 15.0 ms | 5.01× | 60.0 ms | 20.0 ms | 100 MiB | 20 MiB |
| elmstatic-emu 0.19.3 cold | 272.6 ms | 98.4 ms | 2.77× | 660.0 ms | 290.0 ms | 722 MiB | 87 MiB |
| elmstatic-emu 0.19.3 warm | 122.8 ms | 23.4 ms | 5.24× | 110.0 ms | 40.0 ms | 167 MiB | 30 MiB |
| elmstatic-emu 0.19.3 `--optimize` cold | 260.9 ms | 99.6 ms | 2.62× | 640.0 ms | 290.0 ms | 722 MiB | 90 MiB |
| elmstatic-emu 0.19.3 `--optimize` warm | 123.7 ms | 23.5 ms | 5.27× | 110.0 ms | 40.0 ms | 166 MiB | 30 MiB |
| elmstatic-md 0.19.3 cold | 227.2 ms | 80.4 ms | 2.83× | 430.0 ms | 200.0 ms | 511 MiB | 72 MiB |
| elmstatic-md 0.19.3 warm | 93.1 ms | 20.3 ms | 4.59× | 80.0 ms | 30.0 ms | 131 MiB | 25 MiB |
| elmstatic-md 0.19.3 `--optimize` cold | 226.2 ms | 80.4 ms | 2.81× | 440.0 ms | 210.0 ms | 511 MiB | 71 MiB |
| elmstatic-md 0.19.3 `--optimize` warm | 94.0 ms | 20.3 ms | 4.64× | 80.0 ms | 30.0 ms | 131 MiB | 24 MiB |
| emdash 0.19.3 cold | 254.7 ms | 98.1 ms | 2.60× | 550.0 ms | 280.0 ms | 582 MiB | 75 MiB |
| emdash 0.19.3 warm | 114.2 ms | 27.7 ms | 4.13× | 120.0 ms | 60.0 ms | 153 MiB | 31 MiB |
| emdash 0.19.3 `--optimize` cold | 250.7 ms | 96.2 ms | 2.61× | 550.0 ms | 280.0 ms | 581 MiB | 76 MiB |
| emdash 0.19.3 `--optimize` warm | 110.0 ms | 27.6 ms | 3.98× | 120.0 ms | 60.0 ms | 153 MiB | 31 MiB |
| package-elm-lang-org 0.19.3 cold | 196.0 ms | 70.9 ms | 2.76× | 380.0 ms | 200.0 ms | 432 MiB | 61 MiB |
| package-elm-lang-org 0.19.3 warm | 92.0 ms | 19.2 ms | 4.79× | 80.0 ms | 30.0 ms | 121 MiB | 24 MiB |
| package-elm-lang-org 0.19.3 `--optimize` cold | 199.1 ms | 70.0 ms | 2.84× | 380.0 ms | 200.0 ms | 432 MiB | 62 MiB |
| package-elm-lang-org 0.19.3 `--optimize` warm | 91.0 ms | 20.3 ms | 4.49× | 90.0 ms | 30.0 ms | 120 MiB | 24 MiB |
| ui-core-examples 0.19.3 cold | 139.6 ms | 46.6 ms | 2.99× | 200.0 ms | 100.0 ms | 249 MiB | 41 MiB |
| ui-core-examples 0.19.3 warm | 68.8 ms | 11.8 ms | 5.83× | 50.0 ms | 10.0 ms | 88 MiB | 16 MiB |
| ui-core-examples 0.19.3 `--optimize` cold | 142.8 ms | 46.6 ms | 3.06× | 210.0 ms | 100.0 ms | 249 MiB | 40 MiB |
| ui-core-examples 0.19.3 `--optimize` warm | 66.7 ms | 10.7 ms | 6.22× | 50.0 ms | 10.0 ms | 87 MiB | 15 MiB |
| concourse 0.19.3 cold | 704.4 ms | 248.0 ms | 2.84× | 2.12 s | 1.05 s | 2094 MiB | 230 MiB |
| concourse 0.19.3 warm | 244.1 ms | 65.9 ms | 3.71× | 330.0 ms | 200.0 ms | 374 MiB | 76 MiB |
| concourse 0.19.3 `--optimize` cold | 704.0 ms | 247.9 ms | 2.84× | 2.15 s | 1.08 s | 2094 MiB | 246 MiB |
| concourse 0.19.3 `--optimize` warm | 248.4 ms | 67.0 ms | 3.71× | 320.0 ms | 200.0 ms | 374 MiB | 74 MiB |
| guide-repl 0.19.3 cold | 154.4 ms | 55.1 ms | 2.80× | 290.0 ms | 150.0 ms | 339 MiB | 52 MiB |
| guide-repl 0.19.3 warm | 80.5 ms | 16.0 ms | 5.03× | 70.0 ms | 20.0 ms | 103 MiB | 20 MiB |
| guide-repl 0.19.3 `--optimize` cold | 159.6 ms | 54.0 ms | 2.95× | 290.0 ms | 150.0 ms | 338 MiB | 53 MiB |
| guide-repl 0.19.3 `--optimize` warm | 76.2 ms | 16.0 ms | 4.76× | 60.0 ms | 20.0 ms | 103 MiB | 19 MiB |

Both refuse: azimutt 0.19.1, azimutt 0.19.1 `--optimize`, ellie 0.19.1, ellie 0.19.1 `--optimize`, obsidian-card-board 0.19.1, obsidian-card-board 0.19.1 `--optimize`, massivedecks 0.19.1, massivedecks 0.19.1 `--optimize`, elm-analyse 0.19.1, elm-analyse 0.19.1 `--optimize`, elm-shared-state 0.19.1, elm-shared-state 0.19.1 `--optimize`, bots-audio-message 0.19.1, bots-audio-message 0.19.1 `--optimize`, bots-multi-instance 0.19.1, bots-multi-instance 0.19.1 `--optimize`, elm-designer 0.19.1, elm-designer 0.19.1 `--optimize`, textusm 0.19.1, textusm 0.19.1 `--optimize`, unison-local-ui 0.19.1, unison-local-ui 0.19.1 `--optimize`, ui-core-storybook 0.19.1, ui-core-storybook 0.19.1 `--optimize`, elm-charts-docs 0.19.1, elm-charts-docs 0.19.1 `--optimize`, sharry 0.19.1, sharry 0.19.1 `--optimize`, azimutt 0.19.2, azimutt 0.19.2 `--optimize`, ellie 0.19.2, ellie 0.19.2 `--optimize`, obsidian-card-board 0.19.2, obsidian-card-board 0.19.2 `--optimize`, massivedecks 0.19.2, massivedecks 0.19.2 `--optimize`, elm-analyse 0.19.2, elm-analyse 0.19.2 `--optimize`, elm-shared-state 0.19.2, elm-shared-state 0.19.2 `--optimize`, bots-audio-message 0.19.2, bots-audio-message 0.19.2 `--optimize`, bots-multi-instance 0.19.2, bots-multi-instance 0.19.2 `--optimize`, elm-designer 0.19.2, elm-designer 0.19.2 `--optimize`, textusm 0.19.2, textusm 0.19.2 `--optimize`, unison-local-ui 0.19.2, unison-local-ui 0.19.2 `--optimize`, ui-core-storybook 0.19.2, ui-core-storybook 0.19.2 `--optimize`, elm-charts-docs 0.19.2, elm-charts-docs 0.19.2 `--optimize`, sharry 0.19.2, sharry 0.19.2 `--optimize`, azimutt 0.19.3, azimutt 0.19.3 `--optimize`, ellie 0.19.3, ellie 0.19.3 `--optimize`, obsidian-card-board 0.19.3, obsidian-card-board 0.19.3 `--optimize`, massivedecks 0.19.3, massivedecks 0.19.3 `--optimize`, elm-analyse 0.19.3, elm-analyse 0.19.3 `--optimize`, elm-shared-state 0.19.3, elm-shared-state 0.19.3 `--optimize`, bots-audio-message 0.19.3, bots-audio-message 0.19.3 `--optimize`, bots-multi-instance 0.19.3, bots-multi-instance 0.19.3 `--optimize`, elm-designer 0.19.3, elm-designer 0.19.3 `--optimize`, textusm 0.19.3, textusm 0.19.3 `--optimize`, unison-local-ui 0.19.3, unison-local-ui 0.19.3 `--optimize`, ui-core-storybook 0.19.3, ui-core-storybook 0.19.3 `--optimize`, elm-charts-docs 0.19.3, elm-charts-docs 0.19.3 `--optimize`, sharry 0.19.3, sharry 0.19.3 `--optimize`.

### elm make --docs

As for `elm make`, documenting packages with `--docs`: the core library, three widely used packages, the largest corpus package by modules and one of the largest by bytes, with the same `docs.json` from both. A package's `elm.json` admits a range of releases, and nomi builds it as the newest of them does, so only the newest elm is measured. The best of 3 runs counts for each measure.

| workload | elm | nomi | speedup | elm CPU | nomi CPU | elm memory | nomi memory |
|---|--:|--:|--:|--:|--:|--:|--:|
| elm/core 1.0.5, 0.19.3 cold | 81.5 ms | 20.3 ms | 4.02× | 100.0 ms | 30.0 ms | 128 MiB | 24 MiB |
| elm/core 1.0.5, 0.19.3 warm | 78.4 ms | 16.0 ms | 4.89× | 90.0 ms | 30.0 ms | 122 MiB | 21 MiB |
| mdgriffith/elm-ui 1.1.8, 0.19.3 cold | 273.5 ms | 76.5 ms | 3.58× | 420.0 ms | 200.0 ms | 453 MiB | 63 MiB |
| mdgriffith/elm-ui 1.1.8, 0.19.3 warm | 101.6 ms | 37.2 ms | 2.73× | 110.0 ms | 60.0 ms | 142 MiB | 37 MiB |
| rtfeldman/elm-css 18.0.0, 0.19.3 cold | 302.3 ms | 79.6 ms | 3.80× | 500.0 ms | 230.0 ms | 558 MiB | 69 MiB |
| rtfeldman/elm-css 18.0.0, 0.19.3 warm | 189.4 ms | 58.3 ms | 3.25× | 230.0 ms | 100.0 ms | 299 MiB | 58 MiB |
| dillonkearns/elm-graphql 5.0.12, 0.19.3 cold | 168.3 ms | 56.1 ms | 3.00× | 330.0 ms | 170.0 ms | 354 MiB | 50 MiB |
| dillonkearns/elm-graphql 5.0.12, 0.19.3 warm | 77.2 ms | 17.1 ms | 4.51× | 70.0 ms | 30.0 ms | 93 MiB | 24 MiB |
| finos/morphir-elm 22.0.2, 0.19.3 cold | 35.38 s | 210.3 ms | 168.29× | 37.79 s | 1.33 s | 3244 MiB | 263 MiB |
| finos/morphir-elm 22.0.2, 0.19.3 warm | 232.9 ms | 96.4 ms | 2.42× | 410.0 ms | 310.0 ms | 470 MiB | 132 MiB |
| icidasset/elm-material-icons 11.0.0, 0.19.3 cold | 922.2 ms | 329.6 ms | 2.80× | 2.02 s | 820.0 ms | 2829 MiB | 548 MiB |
| icidasset/elm-material-icons 11.0.0, 0.19.3 warm | 856.5 ms | 304.4 ms | 2.81× | 1.89 s | 750.0 ms | 2713 MiB | 534 MiB |

### elm-format

nomi, run as `elm-format`, against the pinned elm-format 0.8.8 (its official build, with its default runtime options), from process start to exit. Each corpus app is validated whole with `--validate`, which reads and formats every module and writes nothing; its largest module is formatted through `--stdin`, as editors do, with the same output from both. The best of 5 runs counts for each measure.

| workload | elm-format | nomi | speedup | elm-format CPU | nomi CPU | elm-format memory | nomi memory |
|---|--:|--:|--:|--:|--:|--:|--:|
| exosphere `--validate`, 218 modules, 1940 KiB | 433.1 ms | 27.7 ms | 15.66× | 4.60 s | 260.0 ms | 160 MiB | 50 MiB |
| exosphere `--stdin`, 191 KiB | 134.7 ms | 17.1 ms | 7.89× | 220.0 ms | 10.0 ms | 29 MiB | 14 MiB |
| liascript `--validate`, 189 modules, 1361 KiB | 325.9 ms | 21.8 ms | 14.93× | 3.76 s | 240.0 ms | 167 MiB | 50 MiB |
| liascript `--stdin`, 127 KiB | 93.4 ms | 11.8 ms | 7.93× | 140.0 ms | 0.0 ms | 21 MiB | 12 MiB |
| docspell `--validate`, 583 modules, 2478 KiB | 616.3 ms | 49.5 ms | 12.44× | 7.39 s | 430.0 ms | 160 MiB | 48 MiB |
| docspell `--stdin`, 104 KiB | 103.6 ms | 17.1 ms | 6.07× | 130.0 ms | 0.0 ms | 21 MiB | 13 MiB |
| elm-3d-scene `--validate`, 59 modules, 929 KiB | 194.7 ms | 16.4 ms | 11.88× | 1.57 s | 120.0 ms | 136 MiB | 41 MiB |
| elm-3d-scene `--stdin`, 132 KiB | 63.5 ms | 4.4 ms | 14.36× | 60.0 ms | 0.0 ms | 20 MiB | 7 MiB |
| elm-spa-example `--validate`, 34 modules, 148 KiB | 69.3 ms | 7.0 ms | 9.92× | 330.0 ms | 30.0 ms | 95 MiB | 18 MiB |
| elm-spa-example `--stdin`, 18 KiB | 42.4 ms | 5.4 ms | 7.82× | 40.0 ms | 0.0 ms | 13 MiB | 7 MiB |
| azimutt `--validate`, 474 modules, 2373 KiB | 585.6 ms | 35.2 ms | 16.66× | 7.60 s | 490.0 ms | 171 MiB | 62 MiB |
| azimutt `--stdin`, 113 KiB | 83.6 ms | 10.7 ms | 7.78× | 100.0 ms | 0.0 ms | 19 MiB | 11 MiB |
| elm-lang-org `--validate`, 116 modules, 603 KiB | 122.7 ms | 9.8 ms | 12.59× | 800.0 ms | 60.0 ms | 113 MiB | 23 MiB |
| elm-lang-org `--stdin`, 28 KiB | 53.0 ms | 6.5 ms | 8.10× | 60.0 ms | 0.0 ms | 13 MiB | 8 MiB |
| elm-lang-org-editor `--validate`, 116 modules, 603 KiB | 120.2 ms | 10.5 ms | 11.40× | 800.0 ms | 70.0 ms | 112 MiB | 23 MiB |
| elm-lang-org-editor `--stdin`, 28 KiB | 53.0 ms | 6.6 ms | 8.07× | 60.0 ms | 0.0 ms | 13 MiB | 8 MiB |
| todomvc `--validate`, 1 modules, 10 KiB | 31.8 ms | 5.5 ms | 5.83× | 20.0 ms | 0.0 ms | 12 MiB | 8 MiB |
| todomvc `--stdin`, 10 KiB | 32.9 ms | 4.4 ms | 7.43× | 20.0 ms | 0.0 ms | 12 MiB | 7 MiB |
| ellie `--validate`, 96 modules, 381 KiB | 120.1 ms | 10.8 ms | 11.12× | 950.0 ms | 70.0 ms | 117 MiB | 24 MiB |
| ellie `--stdin`, 32 KiB | 42.5 ms | 6.5 ms | 6.50× | 40.0 ms | 0.0 ms | 14 MiB | 8 MiB |
| obsidian-card-board `--validate`, 135 modules, 1255 KiB | 273.0 ms | 19.2 ms | 14.19× | 2.68 s | 180.0 ms | 145 MiB | 47 MiB |
| obsidian-card-board `--stdin`, 89 KiB | 83.5 ms | 8.7 ms | 9.65× | 110.0 ms | 0.0 ms | 20 MiB | 9 MiB |
| nixos-search `--validate`, 16 modules, 229 KiB | 158.9 ms | 10.7 ms | 14.78× | 500.0 ms | 30.0 ms | 78 MiB | 21 MiB |
| nixos-search `--stdin`, 54 KiB | 73.1 ms | 8.7 ms | 8.43× | 100.0 ms | 0.0 ms | 16 MiB | 9 MiB |
| kite `--validate`, 23 modules, 308 KiB | 198.8 ms | 22.3 ms | 8.91× | 510.0 ms | 40.0 ms | 93 MiB | 26 MiB |
| kite `--stdin`, 163 KiB | 174.4 ms | 20.2 ms | 8.62× | 260.0 ms | 10.0 ms | 28 MiB | 15 MiB |
| massivedecks `--validate`, 169 modules, 986 KiB | 233.9 ms | 18.1 ms | 12.94× | 2.37 s | 170.0 ms | 163 MiB | 50 MiB |
| massivedecks `--stdin`, 55 KiB | 73.0 ms | 9.6 ms | 7.57× | 70.0 ms | 0.0 ms | 16 MiB | 9 MiB |
| gingko `--validate`, 76 modules, 962 KiB | 356.9 ms | 31.1 ms | 11.47× | 2.11 s | 140.0 ms | 150 MiB | 48 MiB |
| gingko `--stdin`, 316 KiB | 234.6 ms | 25.5 ms | 9.20× | 390.0 ms | 10.0 ms | 35 MiB | 19 MiB |
| elm-analyse `--validate`, 166 modules, 427 KiB | 111.7 ms | 9.2 ms | 12.17× | 1.12 s | 80.0 ms | 115 MiB | 20 MiB |
| elm-analyse `--stdin`, 15 KiB | 31.9 ms | 4.4 ms | 7.21× | 20.0 ms | 0.0 ms | 12 MiB | 7 MiB |
| sanderling `--validate`, 15 modules, 364 KiB | 147.2 ms | 17.1 ms | 8.63× | 410.0 ms | 40.0 ms | 77 MiB | 24 MiB |
| sanderling `--stdin`, 136 KiB | 113.2 ms | 16.0 ms | 7.07× | 170.0 ms | 10.0 ms | 23 MiB | 13 MiB |
| elmboy `--validate`, 46 modules, 283 KiB | 82.5 ms | 8.7 ms | 9.53× | 570.0 ms | 50.0 ms | 114 MiB | 24 MiB |
| elmboy `--stdin`, 37 KiB | 42.4 ms | 5.5 ms | 7.74× | 30.0 ms | 0.0 ms | 13 MiB | 8 MiB |
| elm-shared-state `--validate`, 10 modules, 28 KiB | 31.9 ms | 4.4 ms | 7.30× | 50.0 ms | 0.0 ms | 41 MiB | 9 MiB |
| elm-shared-state `--stdin`, 5 KiB | 31.8 ms | 3.4 ms | 9.47× | 10.0 ms | 0.0 ms | 11 MiB | 6 MiB |
| spades `--validate`, 30 modules, 43 KiB | 31.9 ms | 4.8 ms | 6.70× | 110.0 ms | 10.0 ms | 78 MiB | 10 MiB |
| spades `--stdin`, 4 KiB | 22.3 ms | 3.3 ms | 6.71× | 0.0 ms | 0.0 ms | 12 MiB | 6 MiB |
| swarm-dashboard `--validate`, 8 modules, 32 KiB | 35.0 ms | 5.5 ms | 6.41× | 60.0 ms | 0.0 ms | 39 MiB | 10 MiB |
| swarm-dashboard `--stdin`, 6 KiB | 22.3 ms | 3.4 ms | 6.66× | 10.0 ms | 0.0 ms | 11 MiB | 7 MiB |
| pomodorolm `--validate`, 11 modules, 101 KiB | 96.4 ms | 7.6 ms | 12.74× | 220.0 ms | 10.0 ms | 52 MiB | 14 MiB |
| pomodorolm `--stdin`, 30 KiB | 42.4 ms | 5.5 ms | 7.75× | 40.0 ms | 0.0 ms | 14 MiB | 7 MiB |
| bots-audio-message `--validate`, 133 modules, 2450 KiB | 429.1 ms | 29.9 ms | 14.37× | 4.70 s | 300.0 ms | 216 MiB | 67 MiB |
| bots-audio-message `--stdin`, 136 KiB | 113.2 ms | 16.1 ms | 7.04× | 170.0 ms | 10.0 ms | 23 MiB | 13 MiB |
| bots-tribal-wars `--validate`, 133 modules, 2450 KiB | 439.7 ms | 29.1 ms | 15.09× | 4.79 s | 290.0 ms | 214 MiB | 67 MiB |
| bots-tribal-wars `--stdin`, 136 KiB | 113.3 ms | 16.0 ms | 7.07× | 160.0 ms | 0.0 ms | 23 MiB | 13 MiB |
| bots-multi-instance `--validate`, 133 modules, 2450 KiB | 420.9 ms | 28.1 ms | 14.99× | 4.69 s | 290.0 ms | 213 MiB | 67 MiB |
| bots-multi-instance `--stdin`, 136 KiB | 113.1 ms | 16.1 ms | 7.05× | 170.0 ms | 10.0 ms | 23 MiB | 13 MiB |
| elm-designer `--validate`, 27 modules, 363 KiB | 202.4 ms | 14.5 ms | 13.99× | 740.0 ms | 60.0 ms | 112 MiB | 30 MiB |
| elm-designer `--stdin`, 88 KiB | 173.3 ms | 11.9 ms | 14.61× | 240.0 ms | 0.0 ms | 20 MiB | 11 MiB |
| elmstatic-emu `--validate`, 13 modules, 33 KiB | 36.1 ms | 5.4 ms | 6.63× | 70.0 ms | 0.0 ms | 52 MiB | 10 MiB |
| elmstatic-emu `--stdin`, 4 KiB | 22.4 ms | 3.3 ms | 6.75× | 0.0 ms | 0.0 ms | 12 MiB | 6 MiB |
| elmstatic-md `--validate`, 13 modules, 33 KiB | 35.1 ms | 4.4 ms | 8.01× | 80.0 ms | 10.0 ms | 54 MiB | 11 MiB |
| elmstatic-md `--stdin`, 4 KiB | 22.3 ms | 3.3 ms | 6.67× | 10.0 ms | 0.0 ms | 11 MiB | 6 MiB |
| textusm `--validate`, 210 modules, 1151 KiB | 423.8 ms | 20.2 ms | 21.00× | 3.46 s | 190.0 ms | 139 MiB | 42 MiB |
| textusm `--stdin`, 65 KiB | 73.0 ms | 10.7 ms | 6.80× | 90.0 ms | 0.0 ms | 17 MiB | 10 MiB |
| emdash `--validate`, 30 modules, 229 KiB | 209.7 ms | 11.2 ms | 18.64× | 700.0 ms | 30.0 ms | 106 MiB | 21 MiB |
| emdash `--stdin`, 74 KiB | 73.0 ms | 9.7 ms | 7.54× | 110.0 ms | 0.0 ms | 18 MiB | 10 MiB |
| package-elm-lang-org `--validate`, 15 modules, 69 KiB | 56.9 ms | 7.6 ms | 7.50× | 160.0 ms | 10.0 ms | 63 MiB | 14 MiB |
| package-elm-lang-org `--stdin`, 18 KiB | 42.4 ms | 5.5 ms | 7.77× | 30.0 ms | 0.0 ms | 13 MiB | 8 MiB |
| unison-local-ui `--validate`, 21 modules, 91 KiB | 47.2 ms | 6.5 ms | 7.25× | 160.0 ms | 10.0 ms | 78 MiB | 14 MiB |
| unison-local-ui `--stdin`, 19 KiB | 32.9 ms | 4.4 ms | 7.48× | 20.0 ms | 0.0 ms | 12 MiB | 7 MiB |
| ui-core-examples `--validate`, 190 modules, 1084 KiB | 243.7 ms | 18.9 ms | 12.87× | 2.51 s | 180.0 ms | 142 MiB | 41 MiB |
| ui-core-examples `--stdin`, 147 KiB | 83.6 ms | 9.7 ms | 8.62× | 90.0 ms | 0.0 ms | 19 MiB | 10 MiB |
| ui-core-storybook `--validate`, 190 modules, 1084 KiB | 230.8 ms | 18.2 ms | 12.65× | 2.36 s | 190.0 ms | 137 MiB | 41 MiB |
| ui-core-storybook `--stdin`, 147 KiB | 73.1 ms | 9.7 ms | 7.55× | 100.0 ms | 0.0 ms | 19 MiB | 10 MiB |
| elm-charts-docs `--validate`, 289 modules, 1908 KiB | 406.8 ms | 27.4 ms | 14.87× | 2.19 s | 220.0 ms | 196 MiB | 48 MiB |
| elm-charts-docs `--stdin`, 925 KiB | 148.0 ms | 17.1 ms | 8.68× | 150.0 ms | 0.0 ms | 82 MiB | 18 MiB |
| sharry `--validate`, 120 modules, 532 KiB | 171.8 ms | 12.2 ms | 14.08× | 1.50 s | 90.0 ms | 125 MiB | 28 MiB |
| sharry `--stdin`, 26 KiB | 32.9 ms | 5.5 ms | 6.03× | 30.0 ms | 0.0 ms | 13 MiB | 7 MiB |
| concourse `--validate`, 176 modules, 2409 KiB | 607.1 ms | 37.8 ms | 16.07× | 6.00 s | 350.0 ms | 177 MiB | 71 MiB |
| concourse `--stdin`, 198 KiB | 164.2 ms | 19.2 ms | 8.54× | 260.0 ms | 10.0 ms | 27 MiB | 16 MiB |
| guide-repl `--validate`, 3 modules, 25 KiB | 43.5 ms | 7.5 ms | 5.77× | 50.0 ms | 0.0 ms | 22 MiB | 9 MiB |
| guide-repl `--stdin`, 19 KiB | 42.5 ms | 6.5 ms | 6.52× | 40.0 ms | 0.0 ms | 13 MiB | 8 MiB |

### language server

nomi's server against elm-language-server, each spoken to over its standard streams as an editor would, on corpus apps small to large. Once the server has started and gone quiet, a session opens the app's entry module with a type error added, which both report on open (first diagnostics); saves it fixed and broken again, 6 times (save to diagnostics); and, saved fixed once more, asks for completions in it, 6 times. The times are the best of each, from the message sent to the answer received. CPU time is the server's and its child processes', and memory the peak of their resident sets together, sampled every 20 ms from the session's start to the end of that part. Both run offline over the same package sources, with the replay suite's settings: diagnostics on open and save.

| workload | elm-language-server | nomi | speedup | elm-language-server CPU | nomi CPU | elm-language-server memory | nomi memory |
|---|--:|--:|--:|--:|--:|--:|--:|
| todomvc: first diagnostics | 157.4 ms | 42.1 ms | 3.74× | 740.0 ms | 200.0 ms | 191 MiB | 64 MiB |
| todomvc: save to diagnostics | 81.4 ms | 4.8 ms | 17.07× | 91.7 ms | 13.3 ms | 214 MiB | 70 MiB |
| todomvc: completion | 37.8 ms | 22.5 ms | 1.68× | 76.7 ms | 11.7 ms | 214 MiB | 85 MiB |
| elm-spa-example: first diagnostics | 243.9 ms | 71.7 ms | 3.40× | 1.00 s | 350.0 ms | 272 MiB | 81 MiB |
| elm-spa-example: save to diagnostics | 85.0 ms | 7.1 ms | 11.92× | 103.3 ms | 31.7 ms | 272 MiB | 86 MiB |
| elm-spa-example: completion | 31.7 ms | 23.6 ms | 1.34× | 86.7 ms | 15.0 ms | 272 MiB | 103 MiB |
| exosphere: first diagnostics | 2.20 s | 1.53 s | 1.44× | 6.78 s | 5.34 s | 3588 MiB | 919 MiB |
| exosphere: save to diagnostics | 125.7 ms | 20.9 ms | 6.01× | 175.0 ms | 91.7 ms | 3588 MiB | 919 MiB |
| exosphere: completion | 28.4 ms | 22.0 ms | 1.29× | 71.7 ms | 25.0 ms | 3588 MiB | 919 MiB |

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
