// nomi's, not node-test-runner's: one worker running the program as
// elm-test runs it in its own process when it has one worker
// (`Supervisor.run` with `processes === 1`). An exception loading or
// starting it shows as its message alone, as elm-test's `catch` shows it.
// Run as `node -e <this> <program> <index> <port>`.

var net = require('net');
var client = net.createConnection({
  host: '127.0.0.1',
  port: Number(process.argv[3]),
});

client.on('error', function (error) {
  console.error(error);
  client.end();
  process.exit(1);
});

client.setEncoding('utf8');
client.setNoDelay(true);

var send;
try {
  send = require(process.argv[1]).run(0, function (msg) {
    client.write(JSON.stringify(msg) + '\n');
  });
} catch (error) {
  console.error(error.message);
  process.exit(1);
}

client.on('data', function (msg) {
  send(JSON.parse(msg));
});
