/*

import Elm.Kernel.NomiHttpServer exposing (loopbackAnswer, loopbackResponse)
import Elm.Kernel.NomiNet exposing (allows, key)
import Elm.Kernel.NomiStream exposing (bytes, view)
import Elm.Kernel.Utils exposing (Tuple0)
import Maybe exposing (Just, Nothing)
import Nomi.Http.WebSocket as WebSocket exposing (Text, Binary, Closed, BadUrl, NotAllowed, Unreachable, Refused)

*/


// CONNECTIONS

// A connection: the messages it received that no fiber took yet, oldest
// first; the fibers waiting for one, and those waiting for it to drain;
// how it closed, once it did; whether a fiber waits for the host rather
// than another fiber; and its transport, `send(data)` saying whether the
// message went (`sent`), waits in Bun's buffer (`pressure`) or was
// dropped, and `close(code, reason)`.
function _NomiWebSocket_connection(host)
{
	return { inbox: [], waiting: [], draining: [], closed: null, host: host, transport: null, fiber: null };
}

// A message the peer sent, as Elm's `Message`.
function _NomiWebSocket_received(connection, data)
{
	var message = typeof data === 'string'
		? __WebSocket_Text(data)
		: __WebSocket_Binary(__NomiStream_bytes(new Uint8Array(data.buffer || data, data.byteOffset || 0, data.byteLength)));
	connection.inbox.push(message);
	_NomiWebSocket_tell(connection);
}

// The connection closed, with the code and reason the close gave: what it
// received is still there to take, and then each fiber waiting is told.
function _NomiWebSocket_ended(connection, code, reason)
{
	connection.closed = connection.closed || { __$code: code, __$reason: reason || '' };
	_NomiWebSocket_tell(connection);
	connection.draining.splice(0).forEach(function(resume) {
		resume(_Runtime.fail(__WebSocket_Closed(connection.closed)));
	});
}

function _NomiWebSocket_drained(connection)
{
	connection.draining.splice(0).forEach(function(resume) {
		resume(_Runtime.succeed(__Utils_Tuple0));
	});
}

function _NomiWebSocket_tell(connection)
{
	while (connection.waiting.length > 0 && (connection.inbox.length > 0 || connection.closed !== null))
	{
		var resume = connection.waiting.shift();
		resume(connection.inbox.length > 0
			? _Runtime.succeed(connection.inbox.shift())
			: _Runtime.fail(__WebSocket_Closed(connection.closed)));
	}
}

function _NomiWebSocket_receive(connection)
{
	var wait = connection.host ? _Runtime.waitHost : _Runtime.wait;
	return wait(function(resume) {
		connection.waiting.push(resume);
		_NomiWebSocket_tell(connection);
		return function() {
			var at = connection.waiting.indexOf(resume);
			at < 0 || connection.waiting.splice(at, 1);
		};
	});
}

// Sends the message, succeeding once the transport took it, or, when it
// waits in Bun's buffer, once that drained.
var _NomiWebSocket_send = F2(function(message, connection)
{
	return _Runtime.andThen(_Runtime.sync(function() {
		if (connection.closed !== null) return 'closed';
		var data = typeof message.a === 'string' ? message.a : __NomiStream_view(message.a);
		return connection.transport.send(data);
	}), function(sent) {
		if (sent === 'sent') return _Runtime.succeed(__Utils_Tuple0);
		if (sent !== 'pressure') return _Runtime.fail(__WebSocket_Closed(connection.closed || { __$code: 1006, __$reason: '' }));
		var wait = connection.host ? _Runtime.waitHost : _Runtime.wait;
		return wait(function(resume) {
			connection.draining.push(resume);
			return function() {
				var at = connection.draining.indexOf(resume);
				at < 0 || connection.draining.splice(at, 1);
			};
		});
	});
});

var _NomiWebSocket_close = F3(function(code, reason, connection)
{
	return _Runtime.sync(function() {
		connection.closed === null && connection.transport.close(code, reason);
		return __Utils_Tuple0;
	});
});

function _NomiWebSocket_closedWith(connection)
{
	return _Runtime.sync(function() {
		return connection.closed === null ? __Maybe_Nothing : __Maybe_Just(connection.closed);
	});
}


// BUN'S SOCKETS

// The transport of a socket Bun's server upgraded.
function _NomiWebSocket_onServer(connection, ws)
{
	connection.transport = {
		send: function(data) {
			var sent = ws.send(data);
			return sent > 0 ? 'sent' : sent === -1 ? 'pressure' : 'dropped';
		},
		close: function(code, reason) { ws.close(code, reason); }
	};
}

// The handlers Bun's server calls for its sockets, each socket's data its
// connection.
var _NomiWebSocket_handlers = {
	open: function(ws) {
		var connection = ws.data;
		_NomiWebSocket_onServer(connection, ws);
		connection.opened && connection.opened();
	},
	message: function(ws, data) { _NomiWebSocket_received(ws.data, data); },
	drain: function(ws) { _NomiWebSocket_drained(ws.data); },
	close: function(ws, code, reason) {
		var connection = ws.data;
		_NomiWebSocket_ended(connection, code, reason);
		connection.left && connection.left();
	}
};


// CLIENTS

// A connection to the URL, the fiber's that made it: once the fiber
// ends, it is closed. On a loopback, the server's handler answers it.
var _NomiWebSocket_connect = F2(function(address, net)
{
	var url;
	try
	{
		url = new URL(address);
	}
	catch (e)
	{
		return _Runtime.fail(__WebSocket_BadUrl(address));
	}
	if (url.protocol !== 'ws:' && url.protocol !== 'wss:')
	{
		return _Runtime.fail(__WebSocket_BadUrl(address));
	}
	if (!__NomiNet_allows(net, url.hostname))
	{
		return _Runtime.fail(__WebSocket_NotAllowed(url.hostname));
	}
	var opened = net.loopback !== null ? _NomiWebSocket_onLoopback(net.loopback, url) : _NomiWebSocket_dial(url);
	return _Runtime.acquireRelease(opened, function(connection) {
		return _Runtime.sync(function() {
			connection.closed === null && connection.transport.close(1000, '');
			return __Utils_Tuple0;
		});
	});
});

// Bun's client socket, once it opened.
function _NomiWebSocket_dial(url)
{
	return _Runtime.async(function(resume) {
		var connection = _NomiWebSocket_connection(true);
		var ws = new WebSocket(url.href);
		ws.binaryType = 'arraybuffer';
		var open = false;
		connection.transport = {
			send: function(data) {
				ws.send(data);
				return 'sent';
			},
			close: function(code, reason) { ws.close(code, reason); }
		};
		ws.onopen = function() {
			open = true;
			resume(_Runtime.succeed(connection));
		};
		ws.onmessage = function(event) { _NomiWebSocket_received(connection, event.data); };
		ws.onclose = function(event) {
			if (!open) return resume(_Runtime.fail(event.reason === 'Expected 101 status code'
				? __WebSocket_Refused
				: __WebSocket_Unreachable(url.host + ': the connection was refused or closed before it opened')));
			_NomiWebSocket_ended(connection, event.code, event.reason);
		};
		return function() { ws.close(); };
	});
}

// A pair of connections in memory, each the other's peer.
function _NomiWebSocket_pair()
{
	var client = _NomiWebSocket_connection(false);
	var server = _NomiWebSocket_connection(false);
	var link = function(from, to) {
		from.transport = {
			send: function(data) {
				_NomiWebSocket_received(to, data);
				return 'sent';
			},
			close: function(code, reason) {
				_NomiWebSocket_ended(from, code, reason);
				_NomiWebSocket_ended(to, code, reason);
				to.left && to.left();
			}
		};
	};
	link(client, server);
	link(server, client);
	return { client: client, server: server };
}

// A connection to a loopback's server: its handler's response must
// upgrade the request, and its socket is the pair's other end.
function _NomiWebSocket_onLoopback(loopback, url)
{
	var port = url.port !== '' ? Number(url.port) : url.protocol === 'wss:' ? 443 : 80;
	var server = loopback.servers.get(__NomiNet_key(url.hostname, port));
	if (server === undefined)
	{
		return _Runtime.fail(__WebSocket_Unreachable(url.host + ': connection refused'));
	}
	var headers = new Headers({ upgrade: 'websocket', connection: 'Upgrade' });
	return _Runtime.andThen(__NomiHttpServer_loopbackAnswer(server, 'GET', url.pathname + url.search, headers, null), function(request) {
		return _Runtime.andThen(__NomiHttpServer_loopbackResponse(request), function(response) {
			return response.upgraded
				? _Runtime.succeed(response.upgraded)
				: _Runtime.fail(__WebSocket_Refused);
		});
	});
}
