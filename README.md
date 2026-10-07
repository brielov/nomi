# nomi

nomi is one Rust binary for Elm: compiler, formatter, language server, test runner and package manager, meant as a drop-in replacement for `elm`, `elm-format` and `elm-language-server`. On top of that it adds a Node platform, where `Task` runs on a structured-concurrency fiber runtime, and workspaces, so a frontend and a backend can share Elm code in one repository. Nothing is usable yet; see [docs/architecture.md](docs/architecture.md) for the design and [docs/plan.md](docs/plan.md) for the build order.
