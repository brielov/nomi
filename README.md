# nomi

nomi is one Rust binary for Elm: compiler, formatter, language server, test runner and package manager, meant as a drop-in replacement for `elm`, `elm-format` and `elm-language-server`. On top of that it adds a server platform on Bun, where `Task` runs on a structured-concurrency fiber runtime, and workspaces, so a frontend and a backend can share Elm code in one repository. See [docs/architecture.md](docs/architecture.md) for the design and [docs/plan.md](docs/plan.md) for the build order.

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

`nomi test` is elm-test 0.19.3-0's command line: its options, its `init`, `install` and `make` commands, and its console, JSON and JUnit reports, the same byte for byte for a given seed. nomi finds the tests by their type and builds them itself, so no `elm` is needed, but they run on Bun 1.4 or later, which must be on your `PATH`. `--compiler` is accepted and ignored.

Invoked under the name `elm-test`, nomi is elm-test. Editors and scripts that run elm-test need only a link:

```sh
ln -s "$(command -v nomi)" ~/.local/bin/elm-test
```

## Bun

nomi also builds Elm programs that run on [Bun](https://bun.sh) 1.4 or later: command-line tools, workers and servers. Bun is the only host for them; a built program run under Node says so and exits. Their packages, `nomi/core`, `nomi/server` and `nomi/test`, come with nomi; `nomi add nomi/core` adds one as any other.

```elm
import Nomi
import Nomi.Stdio as Stdio
import Nomi.Stream as Stream
import Task

main : Nomi.Program
main =
    Nomi.program <|
        \env ->
            Stdio.printLine ("hello, " ++ String.join " " env.args) env.stdout
                |> Task.mapError Stream.errorToString
```

```sh
nomi run -e src/Main.elm -- world        # build it for Bun and run it
nomi build src/Main.elm --output=main.mjs
bun main.mjs world
```

A program is a task, run on fibers that are cancelled together and cleaned up in order: `Nomi.Fiber`, `Nomi.Ref` and `Nomi.Queue` for concurrency, `Nomi.Bytes` and `Nomi.Stream` for bytes and streams of them, with backpressure, `Nomi.Fs` and `Nomi.Path` for files, symbolic links and watches inside a capability, `Nomi.Stdio` and `Nomi.Process` for the system, and `Nomi.Server` for HTTP, which on SIGTERM or Ctrl-C finishes the requests in flight before it exits. `nomi.toml`'s `[[entry]]` tables name a project's programs, which `nomi run` and `nomi run --watch` build and run. `nomi/test` runs a program's tasks in tests, on a file system in memory and a virtual clock. Plain Elm programs build for Bun too (`--platform=bun`), and a build that reaches a browser-only package says which path from `main` got there. [examples/todo-server](examples/todo-server) is a small API server.

## Workspaces

A `nomi.toml` with a `[workspace]` makes the directories its globs find, each with its own `elm.json`, one workspace, and a member's own `nomi.toml` lists its programs:

```toml
# nomi.toml
[workspace]
members = ["apps/*", "packages/*"]

# apps/api/nomi.toml
[[entry]]
main = "src/Main.elm"
args = ["--port", "3000"]
```

The members share one resolution, so the apps agree on every package's version, and a dependency named as a member package is built from that member's source. Members that depend on no unpublished member still build with plain `elm`.

```sh
nomi build                # every member; an entry without an output goes to dist/ in its member
nomi test                 # the tests of each member that has them
nomi run --watch          # every entry, rebuilt as files change
nomi run -p api           # one member, by its directory or its package name
nomi add elm/time -p web
```

At the root, `nomi run` starts each Bun entry, its lines prefixed by its member, and serves each browser entry on a port of its own with a dev server. Under `--watch`, a change rebuilds the members it touches and those that depend on them, restarting a program or reloading a page only when its code changed. [examples/fullstack](examples/fullstack) is an api on Bun and a page in the browser sharing a package.

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
