# fullstack

A todo list with Elm at both ends: a server on Bun and a page in the browser, in one nomi workspace, sharing the code that says what a todo is.

```
nomi.toml            [workspace] members = ["apps/*", "packages/*"]
packages/shared      acme/shared: the Todo type and its JSON encoder and decoders
apps/api             the HTTP api, on Bun with nomi/server
apps/web             the todo list page, calling the api with elm/http
apps/landing         a page about the app, which uses no other member
```

## Running it

From this directory, with Bun 1.4 or later:

```
nomi run --watch
```

nomi builds every member, starts the api on port 3000, and serves each page on its own port: the app at <http://localhost:8000> and the landing page at <http://localhost:8001>. Each program's lines are prefixed with its member, and nomi's own with `[nomi]`.

Then edit something. A change rebuilds the members it touches and those that depend on them: the api restarts and the pages reload, but only when their code changed, so a comment restarts nothing. Try renaming the `"done"` field in `packages/shared/src/Todo.elm`, in both the encoder and the decoder: the api restarts, answering with the new name, and the page reloads, reading it. A file under `apps/web/public` reloads the page without a build. A build that fails stops the api, or leaves the page on its last good code, until it's fixed.

`nomi build` builds every member, `nomi test` runs the tests of each member that has them, and `-p` chooses one, by directory or package name: `nomi run -p api`, `nomi test -p acme/shared`.

## What each part shows

- **`nomi.toml`** at the root makes this a workspace. Its members are the directories the globs find, each with its own `elm.json`. nomi resolves them together, so the apps agree on every package's version.
- **`packages/shared`** is a package that is never published: the apps depend on `acme/shared` 1.0.0 in their `elm.json`, and the workspace's member answers for that name, built from its source. Its tests, in `tests/`, check that the decoder reads what the encoder writes.
- **`apps/api`** is a Bun program on nomi/server. Its entry in `nomi.toml` gives its arguments, the port and the origin of the page that calls it. The page is served from another origin, so the api answers with `Access-Control-Allow-Origin` for it, and answers the browser's preflight before a POST of JSON. Its tests answer requests with the handler, through `Nomi.Server.request` and `respond`, with no server listening.
- **`apps/web`** is a browser entry with `static = "public"`: nomi serves the built code at `/Main.js`, the files in `public`, and `public/index.html` for the page, adding a script that reloads it after a rebuild. The page passes the api's address to the program as a flag, so nothing in the Elm code names a port.
- **`apps/landing`** depends on no other member, so plain `elm make src/Main.elm` builds it as well. With no `static` directory, nomi serves a generated page around its code.

The api's port, the page's port and the origin appear in `apps/api/nomi.toml`, `apps/web/nomi.toml` and `apps/web/public/index.html`; to move one, change it in each.
