# nomi

nomi is one Rust binary for Elm: compiler, formatter, language server, test runner and package manager, meant as a drop-in replacement for `elm`, `elm-format` and `elm-language-server`. On top of that it adds a Node platform, where `Task` runs on a structured-concurrency fiber runtime, and workspaces, so a frontend and a backend can share Elm code in one repository. See [docs/architecture.md](docs/architecture.md) for the design and [docs/plan.md](docs/plan.md) for the build order.

So far nomi is a compiler, a formatter, a language server and a test runner. As `elm`, it builds Elm 0.19 projects into the same JavaScript as elm 0.19.1, 0.19.2 and 0.19.3, byte for byte, with the same reports and terminal output, and it installs, diffs, bumps and publishes packages as they do. Every build measured is faster than elm's, cold or warm, by 1.2 to 3.6 times, on less CPU time and under a third of its memory. As `elm-format`, it formats Elm 0.19 modules byte for byte as elm-format 0.8.8 does, five to twenty-four times as fast. As a language server, it answers as elm-language-server 2.10.0 does, with elm's own diagnostics as you type, faster and in less memory on every workload measured. As elm-test, it runs elm-explorations/test suites and reports them byte for byte as elm-test 0.19.3-0 does. The numbers are in [docs/benchmarks.md](docs/benchmarks.md), and where nomi deliberately differs is listed in [DECISIONS.md](DECISIONS.md).

## Install

With Rust 1.89 or later and a C compiler, for the allocator:

```sh
cargo install --locked --path crates/cli
```

## Build

```sh
nomi build src/Main.elm                 # index.html, as elm make writes it
nomi build src/Main.elm --output=main.js
nomi build src/Main.elm --optimize      # or --debug
nomi check                              # report problems, writing no code
```

`nomi make` is the same as `nomi build`. Packages are managed with `nomi new`, `add`, `rm`, `update`, `outdated` and `tree`, and published with `diff`, `bump` and `publish`. nomi keeps what it downloads and builds in its own cache (`NOMI_CACHE`, or `nomi` in your cache directory), never in `elm-stuff` or `~/.elm`.

## As elm

Invoked under the name `elm`, nomi is the `elm` its project's `elm.json` names, 0.19.1, 0.19.2 or 0.19.3: the same commands, options, messages, exit codes and `--report=json`. Build tools, editors and scripts that run elm need only a link:

```sh
ln -s "$(command -v nomi)" ~/.local/bin/elm
```

Put the link ahead of any other `elm` on your `PATH`. `elm repl` and `elm reactor` aren't part of nomi; each says so in a line.

## Format

```sh
nomi format             # every .elm file below the current directory
nomi format src tests   # these files and directories
nomi format --check     # list unformatted files and fail if there are any
nomi format --stdin     # standard input to standard output
```

`nomi fmt` is the same command. Directories are searched for `.elm` files, skipping `elm-stuff`, `node_modules` and `.git`.

## As elm-format

Invoked under the name `elm-format`, nomi is elm-format 0.8.8: the same options, prompts, messages, exit codes and `--validate` JSON. Editors and scripts that run elm-format need only a link:

```sh
ln -s "$(command -v nomi)" ~/.local/bin/elm-format
```

Put the link ahead of any other `elm-format` on your `PATH`. Only Elm 0.19 is supported; `--elm-version=0.18` and modules detected as 0.18 are refused with a message.

## Test

```sh
nomi test                         # every test in tests/
nomi test "src/**/*Tests.elm"     # the tests in these files
nomi test --seed 12345 --fuzz 200 --report json
nomi test --watch                 # again each time the tests change
```

`nomi test` is elm-test 0.19.3-0's command line: its options, its `init`, `install` and `make` commands, and its console, JSON and JUnit reports, the same byte for byte for a given seed. nomi finds the tests by their type and builds them itself, so no `elm` is needed, but they run in Node, which must be on your `PATH`. `--compiler` is accepted and ignored.

Invoked under the name `elm-test`, nomi is elm-test. Editors and scripts that run elm-test need only a link:

```sh
ln -s "$(command -v nomi)" ~/.local/bin/elm-test
```

## Language server

`nomi lsp` speaks the Language Server Protocol on standard input and output, as elm-language-server does: diagnostics from elm itself and from its lint rules, hover, completion, definitions, references, renames, symbols, code actions and formatting. It reads elm-language-server's settings, and needs neither elm nor elm-format installed; only elm-review, where a project configures it and the settings turn it on, runs as a program of its own.

In Helix, add to `languages.toml`:

```toml
[language-server.nomi]
command = "nomi"
args = ["lsp"]

[[language]]
name = "elm"
language-servers = ["nomi"]
```

Any editor that starts a language server by a command can run `nomi lsp` the same way.
