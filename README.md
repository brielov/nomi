# nomi

nomi is one Rust binary for Elm: compiler, formatter, language server, test runner and package manager, meant as a drop-in replacement for `elm`, `elm-format` and `elm-language-server`. On top of that it adds a Node platform, where `Task` runs on a structured-concurrency fiber runtime, and workspaces, so a frontend and a backend can share Elm code in one repository. See [docs/architecture.md](docs/architecture.md) for the design and [docs/plan.md](docs/plan.md) for the build order.

So far nomi is a formatter. It formats Elm 0.19 modules byte for byte as elm-format 0.8.8 does, on every module of the test corpus, and it runs five to twelve times as fast ([docs/benchmarks.md](docs/benchmarks.md)). Where it deliberately differs is listed in [DECISIONS.md](DECISIONS.md).

## Install

With Rust 1.89 or later:

```sh
cargo install --locked --path crates/cli
```

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
