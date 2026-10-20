# todo-server

A small todo API on nomi/core and nomi/http. The todos live in a `Ref`, a background fiber writes them to `data/todos.json` every five seconds when they changed, and on Ctrl-C or SIGTERM the server stops accepting connections, lets the requests in flight finish, and writes the file once more.

Run it from this directory, on Bun 1.4 or later:

```
nomi run
```

It listens on port 8080, or on `PORT` if that is set, `PORT=0` picking a free one, and writes `listening on <port>` to stderr once it does:

```
curl -X POST -d '{"title": "milk"}' localhost:8080/todos   # {"id":1}
curl localhost:8080/todos                                  # {"1":{"title":"milk","done":false}}
curl -X DELETE localhost:8080/todos/1
curl localhost:8080/quote        # a slow dependency, given two seconds
curl localhost:8080/health       # {"inFlight":0}, the requests being answered
```

`nomi test` runs the tests in `tests/`, which load and flush the store on a file system in memory and a virtual clock, and answer requests made in the test with the handlers, a client disconnecting included.
