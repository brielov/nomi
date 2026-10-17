function run(index, receive) {
  var app = Elm.Test.Generated.Main.init({ flags: index });
  // Without this, each run leaks memory in single-threaded mode:
  Elm = null;
  app.ports.elmTestPort__send.subscribe(receive);
  return app.ports.elmTestPort__receive.send;
}

function main() {
  var net = require('net'),
    client = net.createConnection(pipeFilename);

  client.on('error', function (error) {
    console.error(error);
    client.end();
    process.exit(1);
  });

  // nomi: the supervisor is gone, killed or done, so nothing will read
  // what this worker reports; it ends now rather than when its host
  // notices, or never, if a test holds the event loop.
  client.on('close', function () {
    process.exit(1);
  });

  client.setEncoding('utf8');
  client.setNoDelay(true);

  var send = run(Number(process.argv[2]), function (msg) {
    // We split incoming messages on the socket on newlines. The gist is that node
    // is rather unpredictable in whether or not a single `write` will result in a
    // single `on('data')` callback. Sometimes it does, sometimes multiple writes
    // result in a single callback and - worst of all - sometimes a single read
    // results in multiple callbacks, each receiving a piece of the data. The
    // horror.
    client.write(JSON.stringify(msg) + '\n');
  });

  // A message is a line, which may come in pieces or with another.
  var received = '';
  client.on('data', function (chunk) {
    received += chunk;
    var end;
    while ((end = received.indexOf('\n')) !== -1) {
      var line = received.slice(0, end);
      received = received.slice(end + 1);
      send(JSON.parse(line));
    }
  });
}

// For running single-threaded, export the `run` function.
module.exports = {
  run: run,
};

// When running multi-threaded, connect the pipe and run.
if (require.main === module) {
  main();
}
