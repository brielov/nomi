/*

import Elm.Kernel.NomiNet exposing (allows, key)
import Elm.Kernel.NomiStream exposing (reader, give, fail, wanting, bytes, view, writer, error)
import Elm.Kernel.Utils exposing (Tuple0)
import Nomi.Stream as Stream exposing (Closed)
import Nomi.Tcp as Tcp exposing (NotAllowed, NotFound, Refused, AddressInUse, PermissionDenied, Other)

*/


// CONNECTIONS

// A connection: what the peer sends, what to send it, the peer's address,
// and `end()`, which closes it, at most once.
function _NomiTcp_connection(reader, writer, peer, end)
{
	var connection = { reader: reader, writer: writer, peer: peer, ended: false };
	connection.end = function() {
		if (connection.ended) return;
		connection.ended = true;
		end();
	};
	return connection;
}

function _NomiTcp_reader(connection)
{
	return connection.reader;
}

function _NomiTcp_writer(connection)
{
	return connection.writer;
}

function _NomiTcp_peer(connection)
{
	return connection.peer;
}

// `made`, the connection, closed once the fiber that made it ends.
function _NomiTcp_owned(made)
{
	return _Runtime.acquireRelease(made, _NomiTcp_ending);
}

function _NomiTcp_ending(connection)
{
	return _Runtime.sync(function() {
		connection.end();
		return __Utils_Tuple0;
	});
}

function _NomiTcp_address(host, port)
{
	return (host.indexOf(':') >= 0 ? '[' + host + ']' : host) + ':' + port;
}

function _NomiTcp_badPort(port, least)
{
	return port >= least && port <= 65535 && Math.floor(port) === port
		? null
		: _Runtime.fail(__Tcp_Other('port ' + port + ' is out of range: a port is ' + least + ' to 65535'));
}

var _NomiTcp_connect = F3(function(host, port, net)
{
	var bad = _NomiTcp_badPort(port, 1);
	if (bad !== null) return bad;
	if (!__NomiNet_allows(net, host)) return _Runtime.fail(__Tcp_NotAllowed(host));
	var address = _NomiTcp_address(host, port);
	return _NomiTcp_owned(net.loopback !== null
		? _NomiTcp_dialLoopback(net.loopback, __NomiNet_key(host, port), address)
		: _NomiTcp_dial({ hostname: host, port: port }, address, host));
});

// A narrowed network reaches no Unix socket: none is a host it names.
var _NomiTcp_connectUnix = F2(function(path, net)
{
	if (net.hosts !== null) return _Runtime.fail(__Tcp_NotAllowed(path));
	return _NomiTcp_owned(net.loopback !== null
		? _NomiTcp_dialLoopback(net.loopback, 'unix:' + path, path)
		: _NomiTcp_dial({ unix: path }, path, path));
});


// ON SOCKETS

// A connection on Bun's socket, kept as the socket's `data` with whether
// it closed and the write waiting for it to drain. The socket is read
// only while a read waits that what it sent can't answer.
function _NomiTcp_onSocket(socket, peer)
{
	var state = { socket: socket, closed: false, drained: null, connection: null };
	var reader = __NomiStream_reader(true, function(reader) {
		state.closed || (__NomiStream_wanting(reader) ? socket.resume() : socket.pause());
	});
	var writer = __NomiStream_writer({
		write: function(bytes) { return _NomiTcp_write(state, bytes); },
		// `shutdown()` sends the end and keeps reading; Bun's
		// `shutdown(true)` closes the socket.
		close: _Runtime.sync(function() {
			state.closed || socket.shutdown();
			return __Utils_Tuple0;
		}),
		terminal: false,
		size: function() { return null; }
	});
	state.connection = _NomiTcp_connection(reader, writer, peer, function() {
		state.closed || socket.end();
	});
	socket.pause();
	return state;
}

// Writes the bytes, as much as the socket takes at once, and the rest as
// it drains.
function _NomiTcp_write(state, bytes)
{
	return _Runtime.async(function(resume) {
		var rest = __NomiStream_view(bytes);
		var push = function()
		{
			state.drained = null;
			if (state.closed) return resume(_Runtime.fail(__Stream_Closed));
			var wrote = state.socket.write(rest);
			if (wrote < 0) return resume(_Runtime.fail(__Stream_Closed));
			rest = rest.subarray(wrote);
			if (rest.length === 0) return resume(_Runtime.succeed(__Utils_Tuple0));
			state.drained = push;
		};
		push();
		return function() { state.drained = null; };
	});
}

// Bun's handlers for a socket whose `data` `open` makes.
function _NomiTcp_handlers(open)
{
	return {
		open: open,
		data: function(socket, chunk) {
			// A copy, as Bun may reuse the chunk's memory.
			__NomiStream_give(socket.data.connection.reader, __NomiStream_bytes(new Uint8Array(chunk)));
		},
		// The peer ended its side.
		end: function(socket) {
			var reader = socket.data.connection.reader;
			reader.end === null && __NomiStream_give(reader, null);
		},
		drain: function(socket) {
			var drained = socket.data.drained;
			drained && drained();
		},
		close: function(socket, error) {
			var state = socket.data;
			state.closed = true;
			var reader = state.connection.reader;
			if (reader.end === null)
			{
				error ? __NomiStream_fail(reader, { error: __NomiStream_error(error) }) : __NomiStream_give(reader, null);
			}
			state.drained && state.drained();
		},
		error: function(socket, error) {
			var reader = socket.data.connection.reader;
			reader.end === null && __NomiStream_fail(reader, { error: __NomiStream_error(error) });
		}
	};
}

// A connection Bun made to `options`, its peer's address `address`.
function _NomiTcp_dial(options, address, named)
{
	return _Runtime.async(function(resume) {
		var gone = false;
		var handlers = _NomiTcp_handlers(function(socket) {
			socket.data = _NomiTcp_onSocket(socket, address);
		});
		Bun.connect(Object.assign({ socket: handlers }, options)).then(function(socket) {
			gone ? socket.data.connection.end() : resume(_Runtime.succeed(socket.data.connection));
		}, function(error) {
			gone || resume(_Runtime.fail(_NomiTcp_error(error, named, address)));
		});
		return function() { gone = true; };
	});
}

function _NomiTcp_error(error, named, address)
{
	switch (error && error.code)
	{
		case 'ECONNREFUSED':
		case 'ENOENT':
			return __Tcp_Refused(address);
		case 'ENOTFOUND':
		case 'EAI_NONAME':
			return __Tcp_NotFound(named);
		case 'EADDRINUSE':
			return __Tcp_AddressInUse(address);
		case 'EACCES':
		case 'EPERM':
			return __Tcp_PermissionDenied(address);
		default:
			return __Tcp_Other(address + ': ' + (error && error.message || error));
	}
}


// SERVERS

var _NomiTcp_listen = F5(function(host, port, onListen, handle, net)
{
	var address = _NomiTcp_address(host, port);
	if (!net.listen) return _Runtime.fail(__Tcp_PermissionDenied(address));
	var bad = _NomiTcp_badPort(port, 0);
	if (bad !== null) return bad;
	if (net.loopback === null) return _NomiTcp_serve({ hostname: host, port: port }, address, onListen, handle);
	var loopback = net.loopback;
	var taken = port === 0 ? loopback.next++ : port;
	return _NomiTcp_serveLoopback(loopback, __NomiNet_key(host, taken), _NomiTcp_address(host, taken), onListen(taken), handle);
});

var _NomiTcp_listenUnix = F4(function(path, onListen, handle, net)
{
	if (!net.listen) return _Runtime.fail(__Tcp_PermissionDenied(path));
	return net.loopback === null
		? _NomiTcp_serve({ unix: path }, path, function() { return onListen; }, handle)
		: _NomiTcp_serveLoopback(net.loopback, 'unix:' + path, path, onListen, handle);
});

// A server: the connections it accepted that no fiber has taken yet, and
// the accept loop's `resume` while it waits for one.
function _NomiTcp_server()
{
	return { bun: null, accepted: [], taker: null };
}

// Accepts connections on Bun's listener at `options` until the task is
// interrupted. The scope of the task holds the connections' fibers, so
// interrupting it interrupts them and closes their connections, and then
// stops the listener.
function _NomiTcp_serve(options, address, onListen, handle)
{
	var opened = _Runtime.sync(function() {
		var server = _NomiTcp_server();
		try
		{
			server.bun = Bun.listen(Object.assign({
				socket: _NomiTcp_handlers(function(socket) {
					var peer = socket.remoteAddress ? _NomiTcp_address(socket.remoteAddress, socket.remotePort) : address;
					socket.data = _NomiTcp_onSocket(socket, peer);
					_NomiTcp_arrived(server, socket.data.connection);
				})
			}, options));
		}
		catch (error)
		{
			return { error: _NomiTcp_error(error, address, address) };
		}
		return { server: server };
	});
	var listening = _Runtime.acquireRelease(_Runtime.andThen(opened, function(opened) {
		return opened.error ? _Runtime.fail(opened.error) : _Runtime.succeed(opened.server);
	}), function(server) {
		return _Runtime.sync(function() {
			server.bun.stop(true);
			return __Utils_Tuple0;
		});
	});
	return _Runtime.scoped(_Runtime.andThen(listening, function(server) {
		return _Runtime.andThen(onListen(server.bun.port || 0), function() {
			return _NomiTcp_accept(server, handle);
		});
	}));
}

// Accepts the loopback's connections under `key`, its port or path.
function _NomiTcp_serveLoopback(loopback, key, address, told, handle)
{
	var listening = _Runtime.acquireRelease(_Runtime.andThen(_Runtime.succeed(__Utils_Tuple0), function() {
		if (loopback.listeners.has(key) || loopback.servers.has(key)) return _Runtime.fail(__Tcp_AddressInUse(address));
		var server = _NomiTcp_server();
		loopback.listeners.set(key, server);
		return _Runtime.succeed(server);
	}), function() {
		return _Runtime.sync(function() {
			loopback.listeners.delete(key);
			return __Utils_Tuple0;
		});
	});
	return _Runtime.scoped(_Runtime.andThen(listening, function(server) {
		return _Runtime.andThen(told, function() {
			return _NomiTcp_accept(server, handle);
		});
	}));
}

function _NomiTcp_arrived(server, connection)
{
	var taker = server.taker;
	server.taker = null;
	taker ? taker(_Runtime.succeed(connection)) : server.accepted.push(connection);
}

// Takes each connection as it comes, handing it to `handle` on a fiber of
// the server's scope, which closes it once it ends. It waits for the next
// as for another fiber: an idle server holds no test's clock.
function _NomiTcp_accept(server, handle)
{
	var next = _Runtime.wait(function(resume) {
		if (server.accepted.length > 0)
		{
			resume(_Runtime.succeed(server.accepted.shift()));
			return;
		}
		server.taker = resume;
		return function() { server.taker = null; };
	});
	var loop = _Runtime.andThen(next, function(connection) {
		return _Runtime.andThen(_Runtime.fork(_Runtime.ensuring(handle(connection), _NomiTcp_ending(connection))), function() {
			return loop;
		});
	});
	return loop;
}


// ON A LOOPBACK

// A connection to the loopback's listener under `key`, the listener taking
// the other end of a pair of connections in memory.
function _NomiTcp_dialLoopback(loopback, key, address)
{
	return _Runtime.andThen(_Runtime.succeed(__Utils_Tuple0), function() {
		var server = loopback.listeners.get(key);
		if (server === undefined) return _Runtime.fail(__Tcp_Refused(address));
		var pair = _NomiTcp_pair(address, 'loopback');
		_NomiTcp_arrived(server, pair.server);
		return _Runtime.succeed(pair.client);
	});
}

// Two connections in memory, each the other's peer: what one writes the
// other reads, a write taken at once; closing one's writer ends the other's
// reader; and ending one ends both readers, after which writing fails.
function _NomiTcp_pair(clientPeer, serverPeer)
{
	var ends = [__NomiStream_reader(false, function() {}), __NomiStream_reader(false, function() {})];
	var stop = function(reader) {
		reader.end === null && __NomiStream_give(reader, null);
	};
	var side = function(mine, theirs, peer)
	{
		var writer = __NomiStream_writer({
			write: function(bytes) {
				return _Runtime.andThen(_Runtime.succeed(__Utils_Tuple0), function() {
					if (connection.ended || theirs.end !== null) return _Runtime.fail(__Stream_Closed);
					__NomiStream_give(theirs, bytes);
					return _Runtime.succeed(__Utils_Tuple0);
				});
			},
			close: _Runtime.sync(function() {
				stop(theirs);
				return __Utils_Tuple0;
			}),
			terminal: false,
			size: function() { return null; }
		});
		var connection = _NomiTcp_connection(mine, writer, peer, function() {
			stop(theirs);
			stop(mine);
		});
		return connection;
	};
	return { client: side(ends[0], ends[1], clientPeer), server: side(ends[1], ends[0], serverPeer) };
}
