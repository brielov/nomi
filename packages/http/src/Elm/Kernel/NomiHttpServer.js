/*

import Elm.Kernel.List exposing (fromArray, toArray)
import Elm.Kernel.NomiFs exposing (blob)
import Elm.Kernel.NomiNet exposing (key)
import Elm.Kernel.NomiWebSocket exposing (connection, handlers, pair, ended)
import Elm.Kernel.NomiStdio exposing (log)
import Elm.Kernel.NomiStream exposing (reader, held, give, fail, wanting, read, bytes, view)
import Elm.Kernel.Utils exposing (Tuple0, Tuple2)
import Maybe exposing (Just, Nothing, isJust)
import Nomi.Http.Server as Server exposing (AddressInUse, PermissionDenied, Other)
import Nomi.Stream as Stream exposing (TooLarge, Closed, errorToString)

*/


// SERVING

// How much longer than its drain the program's shutdown may take, for the
// finalizers that run after it.
var _NomiHttpServer_margin = 5000;

// A server on `Bun.serve`. Once it is bound, `onListen` runs with the port
// it took, before the accept loop takes the first request. Its requests
// are the fibers of a draining scope, so they outlive the accept loop:
// interrupting `listen` closes the accept loop's scope, which stops the
// server accepting connections, then the draining scope gives the
// requests in flight the drain to finish, and finally closes every
// connection left. A disconnect interrupts its request through the
// runtime running the server, which is a test's under `nomi test`.
var _NomiHttpServer_listen = F3(function(config, handle, net)
{
	if (!net.listen)
	{
		return _Runtime.fail(__Server_PermissionDenied({ __$host: config.__$host, __$port_: config.__$port_ }));
	}
	if (net.loopback !== null)
	{
		return _NomiHttpServer_onLoopback(config, handle, net.loopback);
	}
	var drain = config.__$drainMilliseconds;
	return _Runtime.withRuntime(function(runtime) {
		_Runtime.allowShutdown(runtime, drain + _NomiHttpServer_margin);
		return _Runtime.draining(drain, function(spawn) {
			var opened = _Runtime.acquireRelease(_NomiHttpServer_open(config, runtime), _NomiHttpServer_close);
			return _Runtime.andThen(opened, function(server) {
				var accepting = _Runtime.addFinalizer(function() { return _NomiHttpServer_stop(server); });
				return _Runtime.scoped(_Runtime.andThen(accepting, function() {
					return _Runtime.andThen(config.__$onListen(server.bun.port), function() {
						return _NomiHttpServer_accept(server, spawn, handle);
					});
				}));
			});
		});
	});
});

// A listening server on `runtime`, with the requests it received that no
// fiber has taken yet, the accept loop's `resume` while it waits for one,
// and whether it stopped accepting. A port no socket can have, as 70000,
// fails here, so its message is nomi's whatever the host would say.
function _NomiHttpServer_open(config, runtime)
{
	var port = config.__$port_;
	if (port < 0 || port > 65535)
	{
		return _Runtime.fail(__Server_Other('port ' + port + ' is out of range: a port is 0 to 65535'));
	}
	return _Runtime.async(function(resume) {
		var server = { bun: null, runtime: runtime, requests: [], taker: null, stopped: false };
		var max = config.__$maxBodyBytes;
		try
		{
			var tls = config.__$tls;
			server.bun = Bun.serve({
				port: port,
				hostname: config.__$host,
				tls: __Maybe_isJust(tls) ? { cert: tls.a.__$certificate, key: tls.a.__$key } : undefined,
				// Past `max`, a body is refused as it is read; Bun's own limit
				// stays above it, so the handler still answers.
				maxRequestBodySize: Math.max(max + 1, 128 * 1024 * 1024),
				// Bun closes a request idle for 10 seconds, even one whose
				// handler is still working; a request in flight is the
				// handler's to answer however long it takes.
				fetch: function(req, bun) {
					bun.timeout(req, 0);
					return _NomiHttpServer_received(server, req, max);
				},
				websocket: __NomiWebSocket_handlers
			});
		}
		catch (error)
		{
			resume(_Runtime.fail(_NomiHttpServer_error(error, config)));
			return;
		}
		resume(_Runtime.succeed(server));
	});
}

// The response to `req`, once a fiber gives it. A request that comes once
// the server stopped accepting is answered 503. The client leaving, which
// aborts the request's signal, interrupts its fiber.
function _NomiHttpServer_received(server, req, max)
{
	if (server.stopped)
	{
		return _NomiHttpServer_unavailable();
	}
	return new Promise(function(answer) {
		var request = _NomiHttpServer_incoming(req.method, _NomiHttpServer_target(req.url), req.headers, max);
		request.req = req;
		request.server = server;
		request.answer = answer;
		req.signal.addEventListener('abort', function() {
			if (request.answered) return;
			request.gone = true;
			request.fiber && server.runtime.interrupt(request.fiber);
			answer(new Response(null, { status: 499 }));
		});
		var taker = server.taker;
		server.taker = null;
		taker ? taker(_Runtime.succeed(request)) : server.requests.push(request);
	});
}

// A request as the kernel keeps it, whether received or made in a test.
function _NomiHttpServer_incoming(method, target, headers, max)
{
	return { method: method, target: target, headers: headers, max: max, req: null, text: undefined, reader: null, server: null, fiber: null, gone: false, answered: false, answer: null };
}

// The request target of the absolute URL Bun gives: its path and query.
function _NomiHttpServer_target(url)
{
	var start = url.indexOf('/', url.indexOf('//') + 2);
	return start < 0 ? '/' : url.slice(start);
}

// Takes each request as it comes and spawns its response in the draining
// scope. It waits for the next one as for another fiber, not the host:
// an idle server holds no test's clock, and a request in flight is held
// by its client.
function _NomiHttpServer_accept(server, spawn, handle)
{
	var next = _Runtime.wait(function(resume) {
		if (server.requests.length > 0)
		{
			resume(_Runtime.succeed(server.requests.shift()));
			return;
		}
		server.taker = resume;
		return function() { server.taker = null; };
	});
	var loop = _Runtime.andThen(next, function(request) {
		return _Runtime.andThen(spawn(_NomiHttpServer_respond(handle, request)), function(fiber) {
			request.fiber = fiber;
			request.gone && server.runtime.interrupt(fiber);
			return loop;
		});
	});
	return loop;
}

// Refuses new connections and closes the idle ones. A request that comes
// later on a connection still open, or that no fiber took, is answered
// 503, and the responses still written close their connections, so no
// client sends another on one the drain will close.
function _NomiHttpServer_stop(server)
{
	return _Runtime.sync(function() {
		server.stopped = true;
		server.bun.stop(false);
		server.requests.splice(0).forEach(function(request) { _NomiHttpServer_send(request, _NomiHttpServer_unavailable()); });
		return __Utils_Tuple0;
	});
}

function _NomiHttpServer_unavailable()
{
	return new Response('the server is stopping', {
		status: 503,
		headers: { 'Content-Type': 'text/plain; charset=utf-8', 'Connection': 'close' }
	});
}

// Closes the connections still open once the drain is over. Bun writes a
// response once the promise answering its request settles, on a
// microtask, so the responses given last are written before the next
// macrotask closes their connections.
function _NomiHttpServer_close(server)
{
	return _Runtime.async(function(resume) {
		setImmediate(function() {
			server.bun.stop(true);
			resume(_Runtime.succeed(__Utils_Tuple0));
		});
	});
}

function _NomiHttpServer_error(error, config)
{
	var address = { __$host: config.__$host, __$port_: config.__$port_ };
	switch (error.code)
	{
		case 'EADDRINUSE':
			return __Server_AddressInUse(address);
		case 'EACCES':
			return __Server_PermissionDenied(address);
		default:
			return __Server_Other(error.message);
	}
}

// Answers `request` with the response the handler gives, on a fiber of its
// own whose exit the request's fiber awaits, so a defect is answered here
// and reported once, by `_NomiHttpServer_died`, rather than by the runtime.
// Interrupting the request's fiber interrupts the handler's, and its
// finalizers run before the request's fiber ends.
function _NomiHttpServer_respond(handle, request)
{
	var answered = _Runtime.andThen(_Runtime.andThen(_Runtime.succeed(request), handle), function(response) {
		if (__Maybe_isJust(response.__$upgrade))
		{
			return _NomiHttpServer_upgraded(request, response.__$upgrade.a);
		}
		var streams = __Maybe_isJust(response.__$reader) && !_NomiHttpServer_bodiless[response.__$status];
		return streams
			? _NomiHttpServer_streamed(request, response, response.__$reader.a)
			: _Runtime.sync(function() {
				_NomiHttpServer_send(request, _NomiHttpServer_response(request, response, null));
				return __Utils_Tuple0;
			});
	});
	return _Runtime.andThen(_Runtime.fork(answered), function(fiber) {
		return _Runtime.andThen(_Runtime.awaitFiber(fiber), function(exit) {
			return exit.$ === _Runtime.ExitKind.DEFECT ? _NomiHttpServer_died(request, exit.defect) : _Runtime.succeed(__Utils_Tuple0);
		});
	});
}

// The statuses whose responses have no body, which Fetch refuses one.
var _NomiHttpServer_bodiless = { 101: true, 204: true, 205: true, 304: true };

// `response` as Fetch's `Response`, its body `source` when it streams,
// which refuses what no server could write, as status 999 or a line break
// in a header, with an exception that makes it a defect.
function _NomiHttpServer_response(request, response, source)
{
	var headers = new Headers();
	var given = __List_toArray(response.__$headers);
	for (var i = 0; i < given.length; i++)
	{
		headers.append(given[i].a, given[i].b);
	}
	request.server && request.server.stopped && headers.set('Connection', 'close');
	var status = response.__$status;
	var body = source !== null ? source
		: __Maybe_isJust(response.__$text) ? response.__$text.a
		: __Maybe_isJust(response.__$bytes) ? __NomiStream_view(response.__$bytes.a)
		: __Maybe_isJust(response.__$file) ? response.__$file.a
		: null;
	return new Response(_NomiHttpServer_bodiless[status] ? null : body, { status: status, headers: headers });
}

// Sends `response` with what `reader` gives as its body, on the fiber
// answering the request, which ends once the body is sent. The body is a
// stream Bun pulls from as the client takes what was sent, and each pull
// is a read of `reader`, so a slow client holds back what produces the
// body. A client going away cancels the stream, and ends the fiber; a
// reader failing, or the fiber ending first, cuts the response short.
function _NomiHttpServer_streamed(request, response, reader)
{
	var pull = null;
	var waiting = null;
	var done = false;
	var wake = function() {
		var resume = waiting;
		waiting = null;
		resume && resume(_Runtime.succeed(__Utils_Tuple0));
	};
	var source = new ReadableStream({
		pull: function(controller) {
			return new Promise(function(resolve) {
				pull = { controller: controller, resolve: resolve };
				wake();
			});
		},
		cancel: function() {
			done = true;
			wake();
		}
	}, { highWaterMark: 0 });
	var sent = _Runtime.sync(function() {
		_NomiHttpServer_send(request, _NomiHttpServer_response(request, response, source));
		return __Utils_Tuple0;
	});
	var asked = _Runtime.waitHost(function(resume) {
		if (pull !== null || done)
		{
			resume(_Runtime.succeed(__Utils_Tuple0));
			return;
		}
		waiting = resume;
		return function() { waiting = null; };
	});
	var failed = function(error) {
		return _Runtime.map(
			__NomiStdio_log('nomi: the response to ' + request.method + ' ' + request.target + ' failed: ' + __Stream_errorToString(error)),
			function() { return null; }
		);
	};
	var loop = _Runtime.andThen(asked, function() {
		if (done) return _Runtime.succeed(__Utils_Tuple0);
		var opening = response.__$opening;
		if (__Maybe_isJust(opening))
		{
			response.__$opening = __Maybe_Nothing;
			var asking = pull;
			pull = null;
			asking.controller.enqueue(_NomiHttpServer_encoder.encode(opening.a));
			asking.resolve();
			return loop;
		}
		return _Runtime.andThen(_Runtime.onError(__NomiStream_read(reader), failed), function(chunk) {
			if (done || chunk === null) return _Runtime.succeed(__Utils_Tuple0);
			var asking = pull;
			pull = null;
			if (__Maybe_isJust(chunk))
			{
				asking.controller.enqueue(__NomiStream_view(chunk.a));
				asking.resolve();
				return loop;
			}
			done = true;
			asking.controller.close();
			asking.resolve();
			return _Runtime.succeed(__Utils_Tuple0);
		});
	});
	// However the fiber ends, a body not sent whole is cut short.
	var cut = _Runtime.sync(function() {
		if (!done)
		{
			done = true;
			pull && (pull.controller.error(new Error('the response was cut short')), pull.resolve());
		}
		return __Utils_Tuple0;
	});
	return _Runtime.andThen(sent, function() { return _Runtime.ensuring(loop, cut); });
}

function _NomiHttpServer_send(request, response)
{
	if (request.answered) return;
	request.answered = true;
	request.answer(response);
}

// Answers 500 for a handler that died and logs one line naming the
// request, as `Nomi.log` does.
function _NomiHttpServer_died(request, defect)
{
	_NomiHttpServer_send(request, new Response('internal server error', {
		status: 500,
		headers: { 'Content-Type': 'text/plain; charset=utf-8' }
	}));
	var described = String(defect).replace(/\s*\n\s*/g, ' ');
	return __NomiStdio_log('nomi: request ' + request.method + ' ' + request.target + ' died: ' + described);
}


// TESTING

// The most a request made in a test may hold, in bytes: `defaults`'s.
var _NomiHttpServer_maxBody = 1024 * 1024;

// A request with no connection behind it, whose body is `text`, its
// headers read by Fetch's rules for repeated ones, as a server's are.
var _NomiHttpServer_request = F4(function(method, target, headers, text)
{
	var read = new Headers();
	for (var list = headers; list.b; list = list.b) // WHILE_CONS
	{
		read.append(list.a.a, list.a.b);
	}
	return { method: method, target: target, headers: read, text: text };
});

// Answers a request `_NomiHttpServer_request` made as a server does, giving
// the response the handler gave, new each time the task runs.
var _NomiHttpServer_answer = F2(function(handle, made)
{
	// A streamed body is read as it is sent, as a client would.
	var received = _Runtime.sync(function() {
		var request = _NomiHttpServer_incoming(made.method, made.target, made.headers, _NomiHttpServer_maxBody);
		request.text = made.text;
		request.answer = function(response) { request.recorded = _NomiHttpServer_recorded(response); };
		return request;
	});
	return _Runtime.andThen(received, function(request) {
		return _Runtime.andThen(_NomiHttpServer_respond(handle, request), function() {
			return _Runtime.fromPromise(function() { return request.recorded; });
		});
	});
});

// What a response holds: its status, its headers as Fetch's `Headers`
// lists them (names in lower case and in order, repeated values joined
// with `, `, each `Set-Cookie` apart), and its body. Bun lists every
// `Set-Cookie` last, so they are sorted in here.
function _NomiHttpServer_recorded(response)
{
	return response.text().then(function(body) {
		var headers = Array.from(response.headers).sort(function(a, b) {
			return a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0;
		});
		return {
			__$status: response.status,
			__$headers: __List_fromArray(headers.map(function(header) { return __Utils_Tuple2(header[0], header[1]); })),
			__$body: body
		};
	});
}


// REQUESTS

function _NomiHttpServer_method(request)
{
	return request.method;
}

function _NomiHttpServer_path(request)
{
	var target = request.target;
	var end = target.indexOf('?');
	var segments = (end < 0 ? target : target.slice(0, end)).split('/');
	return __List_fromArray(segments.filter(Boolean).map(_NomiHttpServer_decode));
}

// A path segment percent-decoded, or as it is when it isn't valid.
function _NomiHttpServer_decode(segment)
{
	try
	{
		return decodeURIComponent(segment);
	}
	catch (e)
	{
		return segment;
	}
}

var _NomiHttpServer_query = F2(function(name, request)
{
	var target = request.target;
	var start = target.indexOf('?');
	var value = start < 0 ? null : new URLSearchParams(target.slice(start + 1)).get(name);
	return value === null ? __Maybe_Nothing : __Maybe_Just(value);
});

// Fetch's `Headers` matches names in any case and joins repeated values.
var _NomiHttpServer_header = F2(function(name, request)
{
	var value;
	try
	{
		value = request.headers.get(name);
	}
	catch (e)
	{
		// A name no header could have.
		value = null;
	}
	return value === null ? __Maybe_Nothing : __Maybe_Just(value);
});


// WEBSOCKETS

// Makes the request's connection a WebSocket, Bun's or, on a loopback,
// one of a pair whose other end the client gets, and runs `connected` on
// a fiber of the request's, which the peer leaving interrupts. Once it
// ends, the connection closes. A request no connection can upgrade is
// answered 426.
function _NomiHttpServer_upgraded(request, connected)
{
	var upgraded = _Runtime.async(function(resume) {
		if (request.loopback && (request.headers.get('upgrade') || '').toLowerCase() === 'websocket')
		{
			var both = __NomiWebSocket_pair();
			request.answered = true;
			request.answer({ upgraded: both.client, status: 101 });
			resume(_Runtime.succeed(both.server));
			return;
		}
		var connection = __NomiWebSocket_connection(true);
		connection.opened = function() { resume(_Runtime.succeed(connection)); };
		if (request.loopback || request.req === null || request.server === null || !request.server.bun.upgrade(request.req, { data: connection }))
		{
			_NomiHttpServer_send(request, new Response('this request is not a WebSocket\'s', {
				status: 426,
				headers: { 'Content-Type': 'text/plain; charset=utf-8', 'Upgrade': 'websocket' }
			}));
			resume(_Runtime.succeed(null));
			return;
		}
		request.answered = true;
		request.answer(undefined);
	});
	return _Runtime.andThen(upgraded, function(connection) {
		if (connection === null) return _Runtime.succeed(__Utils_Tuple0);
		return _Runtime.withRuntime(function(runtime) {
			return _Runtime.andThen(_Runtime.fork(connected(connection)), function(fiber) {
				connection.left = function() { runtime.interrupt(fiber); };
				connection.closed !== null && runtime.interrupt(fiber);
				var closing = _Runtime.sync(function() {
					connection.closed === null && connection.transport.close(1000, '');
					return __Utils_Tuple0;
				});
				return _Runtime.ensuring(_Runtime.map(_Runtime.awaitFiber(fiber), function() { return __Utils_Tuple0; }), closing);
			});
		});
	});
}


// LOOPBACKS

// A server on a loopback is its handler, kept under its host and port,
// one the loopback picks for port 0, until the task is interrupted. The
// loopback's clients answer their requests with it.
function _NomiHttpServer_onLoopback(config, handle, loopback)
{
	var port = config.__$port_ === 0 ? loopback.next++ : config.__$port_;
	var address = { __$host: config.__$host, __$port_: port };
	var key = __NomiNet_key(config.__$host, port);
	if (loopback.servers.has(key))
	{
		return _Runtime.fail(__Server_AddressInUse(address));
	}
	var listening = _Runtime.acquireRelease(
		_Runtime.sync(function() {
			loopback.servers.set(key, { handle: handle, max: config.__$maxBodyBytes });
			return key;
		}),
		function(key) {
			return _Runtime.sync(function() {
				loopback.servers.delete(key);
				return __Utils_Tuple0;
			});
		}
	);
	return _Runtime.andThen(listening, function() {
		return _Runtime.andThen(config.__$onListen(port), function() {
			return _Runtime.wait(function() {});
		});
	});
}

// Answers a loopback's request with `server`'s handler, on a fiber of the
// sender's, so the sender ending is the client going away, and succeeds
// with the request, whose response `loopbackResponse` waits for: it
// comes as soon as it is given, its body streaming on.
function _NomiHttpServer_loopbackAnswer(server, method, target, headers, body)
{
	var received = _Runtime.sync(function() {
		var request = _NomiHttpServer_incoming(method, target, headers, server.max);
		request.loopback = true;
		request.bytes = body === null ? new Uint8Array(0) : body;
		request.answer = function(response) {
			request.response = response;
			request.waiter && request.waiter(_Runtime.succeed(response));
		};
		return request;
	});
	return _Runtime.andThen(received, function(request) {
		return _Runtime.map(_Runtime.fork(_NomiHttpServer_respond(server.handle, request)), function(fiber) {
			request.exchange = fiber;
			return request;
		});
	});
}

function _NomiHttpServer_loopbackResponse(request)
{
	return _Runtime.wait(function(resume) {
		if (request.response)
		{
			resume(_Runtime.succeed(request.response));
			return;
		}
		request.waiter = resume;
		return function() { request.waiter = null; };
	});
}


// FILES

// The file at `path` in `fs`, as Bun sends a file: with its size.
var _NomiHttpServer_file = F2(function(path, fs)
{
	return A2(__NomiFs_blob, path, fs);
});


// BODIES

// The request's body as a reader, made once: a test's text, or what the
// client sends, failing with `TooLarge` past the server's `maxBodyBytes`
// (at once when `Content-Length` says so), and with `Closed` when the
// client goes away first.
function _NomiHttpServer_body(request)
{
	if (request.reader === null)
	{
		var max = request.max;
		if (request.text !== undefined || request.bytes !== undefined)
		{
			var given = request.bytes !== undefined ? request.bytes : _NomiHttpServer_encoder.encode(request.text);
			request.reader = given.length > max ? _NomiHttpServer_failed(__Stream_TooLarge) : __NomiStream_held(__NomiStream_bytes(given));
		}
		else if (Number(request.req.headers.get('content-length')) > max)
		{
			request.reader = _NomiHttpServer_failed(__Stream_TooLarge);
		}
		else
		{
			request.reader = request.req.body === null
				? __NomiStream_held(new DataView(new ArrayBuffer(0)))
				: _NomiHttpServer_limited(request.req.body, max);
		}
	}
	return request.reader;
}

var _NomiHttpServer_encoder = new TextEncoder();

function _NomiHttpServer_limit(request)
{
	return request.max;
}

function _NomiHttpServer_failed(error)
{
	var reader = __NomiStream_reader(false, function() {});
	__NomiStream_fail(reader, { error: error });
	return reader;
}

// A reader of `body`, a web stream, which fails with `TooLarge` once it
// gave more than `max` bytes, the rest left unread.
function _NomiHttpServer_limited(body, max)
{
	var source = body.getReader();
	var reading = false;
	var size = 0;
	return __NomiStream_reader(true, function(reader) {
		if (reading || !__NomiStream_wanting(reader)) return;
		reading = true;
		source.read().then(function(read) {
			reading = false;
			if (read.done) return __NomiStream_give(reader, null);
			size += read.value.length;
			if (size > max)
			{
				source.cancel().catch(function() {});
				return __NomiStream_fail(reader, { error: __Stream_TooLarge });
			}
			__NomiStream_give(reader, __NomiStream_bytes(read.value));
		}, function() {
			reading = false;
			__NomiStream_fail(reader, { error: __Stream_Closed });
		});
	});
}
