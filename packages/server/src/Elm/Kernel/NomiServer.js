/*

import Elm.Kernel.List exposing (fromArray, toArray)
import Elm.Kernel.NomiStdio exposing (log)
import Elm.Kernel.Utils exposing (Tuple0, Tuple2)
import Maybe exposing (Just, Nothing)
import Nomi.Server as Server exposing (AddressInUse, PermissionDenied, Other, TooLarge, Disconnected)

*/


// SERVING

// How much longer than its drain the program's shutdown may take, for the
// finalizers that run after it.
var _NomiServer_margin = 5000;

// A server on `Bun.serve`. Once it is bound, `onListen` runs with the port
// it took, before the accept loop takes the first request. Its requests
// are the fibers of a draining scope, so they outlive the accept loop:
// interrupting `listen` closes the accept loop's scope, which stops the
// server accepting connections, then the draining scope gives the
// requests in flight the drain to finish, and finally closes every
// connection left. A disconnect interrupts its request through the
// runtime running the server, which is a test's under `nomi test`.
var _NomiServer_listen = F3(function(config, handle, net)
{
	var drain = config.__$drainMilliseconds;
	return _Runtime.withRuntime(function(runtime) {
		_Runtime.allowShutdown(runtime, drain + _NomiServer_margin);
		return _Runtime.draining(drain, function(spawn) {
			var opened = _Runtime.acquireRelease(_NomiServer_open(config, runtime), _NomiServer_close);
			return _Runtime.andThen(opened, function(server) {
				var accepting = _Runtime.addFinalizer(function() { return _NomiServer_stop(server); });
				return _Runtime.scoped(_Runtime.andThen(accepting, function() {
					return _Runtime.andThen(config.__$onListen(server.bun.port), function() {
						return _NomiServer_accept(server, spawn, handle);
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
function _NomiServer_open(config, runtime)
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
			server.bun = Bun.serve({
				port: port,
				hostname: config.__$host,
				// Past `max`, a body is refused as it is read; Bun's own limit
				// stays above it, so the handler still answers.
				maxRequestBodySize: Math.max(max + 1, 128 * 1024 * 1024),
				// Bun closes a request idle for 10 seconds, even one whose
				// handler is still working; a request in flight is the
				// handler's to answer however long it takes.
				fetch: function(req, bun) {
					bun.timeout(req, 0);
					return _NomiServer_received(server, req, max);
				}
			});
		}
		catch (error)
		{
			resume(_Runtime.fail(_NomiServer_error(error, config)));
			return;
		}
		resume(_Runtime.succeed(server));
	});
}

// The response to `req`, once a fiber gives it. A request that comes once
// the server stopped accepting is answered 503. The client leaving, which
// aborts the request's signal, interrupts its fiber.
function _NomiServer_received(server, req, max)
{
	if (server.stopped)
	{
		return _NomiServer_unavailable();
	}
	return new Promise(function(answer) {
		var request = _NomiServer_incoming(req.method, _NomiServer_target(req.url), req.headers, max);
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
function _NomiServer_incoming(method, target, headers, max)
{
	return { method: method, target: target, headers: headers, max: max, req: null, text: undefined, body: null, server: null, fiber: null, gone: false, answered: false, answer: null };
}

// The request target of the absolute URL Bun gives: its path and query.
function _NomiServer_target(url)
{
	var start = url.indexOf('/', url.indexOf('//') + 2);
	return start < 0 ? '/' : url.slice(start);
}

// Takes each request as it comes and spawns its response in the draining
// scope. It waits for the next one as for another fiber, not the host:
// an idle server holds no test's clock, and a request in flight is held
// by its client.
function _NomiServer_accept(server, spawn, handle)
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
		return _Runtime.andThen(spawn(_NomiServer_respond(handle, request)), function(fiber) {
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
function _NomiServer_stop(server)
{
	return _Runtime.sync(function() {
		server.stopped = true;
		server.bun.stop(false);
		server.requests.splice(0).forEach(function(request) { _NomiServer_send(request, _NomiServer_unavailable()); });
		return __Utils_Tuple0;
	});
}

function _NomiServer_unavailable()
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
function _NomiServer_close(server)
{
	return _Runtime.async(function(resume) {
		setImmediate(function() {
			server.bun.stop(true);
			resume(_Runtime.succeed(__Utils_Tuple0));
		});
	});
}

function _NomiServer_error(error, config)
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
// and reported once, by `_NomiServer_died`, rather than by the runtime.
// Interrupting the request's fiber interrupts the handler's, and its
// finalizers run before the request's fiber ends.
function _NomiServer_respond(handle, request)
{
	var answered = _Runtime.andThen(_Runtime.andThen(_Runtime.succeed(request), handle), function(response) {
		return _Runtime.sync(function() {
			_NomiServer_send(request, _NomiServer_response(request, response));
			return __Utils_Tuple0;
		});
	});
	return _Runtime.andThen(_Runtime.fork(answered), function(fiber) {
		return _Runtime.andThen(_Runtime.awaitFiber(fiber), function(exit) {
			return exit.$ === _Runtime.ExitKind.DEFECT ? _NomiServer_died(request, exit.defect) : _Runtime.succeed(__Utils_Tuple0);
		});
	});
}

// The statuses whose responses have no body, which Fetch refuses one.
var _NomiServer_bodiless = { 101: true, 204: true, 205: true, 304: true };

// `response` as Fetch's `Response`, which refuses what no server could
// write, as status 999 or a line break in a header, with an exception
// that makes it a defect.
function _NomiServer_response(request, response)
{
	var headers = new Headers();
	var given = __List_toArray(response.__$headers);
	for (var i = 0; i < given.length; i++)
	{
		headers.append(given[i].a, given[i].b);
	}
	request.server && request.server.stopped && headers.set('Connection', 'close');
	var status = response.__$status;
	return new Response(_NomiServer_bodiless[status] ? null : response.__$body, { status: status, headers: headers });
}

function _NomiServer_send(request, response)
{
	if (request.answered) return;
	request.answered = true;
	request.answer(response);
}

// Answers 500 for a handler that died and logs one line naming the
// request, as `Nomi.log` does.
function _NomiServer_died(request, defect)
{
	_NomiServer_send(request, new Response('internal server error', {
		status: 500,
		headers: { 'Content-Type': 'text/plain; charset=utf-8' }
	}));
	var described = String(defect).replace(/\s*\n\s*/g, ' ');
	return __NomiStdio_log('nomi: request ' + request.method + ' ' + request.target + ' died: ' + described);
}


// TESTING

// The most a request made in a test may hold, in bytes: `defaults`'s.
var _NomiServer_maxBody = 1024 * 1024;

// A request with no connection behind it, whose body is `text`, its
// headers read by Fetch's rules for repeated ones, as a server's are.
var _NomiServer_request = F4(function(method, target, headers, text)
{
	var read = new Headers();
	for (var list = headers; list.b; list = list.b) // WHILE_CONS
	{
		read.append(list.a.a, list.a.b);
	}
	return { method: method, target: target, headers: read, text: text };
});

// Answers a request `_NomiServer_request` made as a server does, giving
// the response the handler gave, new each time the task runs.
var _NomiServer_answer = F2(function(handle, made)
{
	var received = _Runtime.sync(function() {
		var request = _NomiServer_incoming(made.method, made.target, made.headers, _NomiServer_maxBody);
		request.text = made.text;
		request.answer = function(response) { request.response = response; };
		return request;
	});
	return _Runtime.andThen(received, function(request) {
		return _Runtime.andThen(_NomiServer_respond(handle, request), function() {
			return _Runtime.fromPromise(function() { return _NomiServer_recorded(request.response); });
		});
	});
});

// What a response holds: its status, its headers as Fetch's `Headers`
// lists them (names in lower case and in order, repeated values joined
// with `, `, each `Set-Cookie` apart), and its body. Bun lists every
// `Set-Cookie` last, so they are sorted in here.
function _NomiServer_recorded(response)
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

function _NomiServer_method(request)
{
	return request.method;
}

function _NomiServer_path(request)
{
	var target = request.target;
	var end = target.indexOf('?');
	var segments = (end < 0 ? target : target.slice(0, end)).split('/');
	return __List_fromArray(segments.filter(Boolean).map(_NomiServer_decode));
}

// A path segment percent-decoded, or as it is when it isn't valid.
function _NomiServer_decode(segment)
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

var _NomiServer_query = F2(function(name, request)
{
	var target = request.target;
	var start = target.indexOf('?');
	var value = start < 0 ? null : new URLSearchParams(target.slice(start + 1)).get(name);
	return value === null ? __Maybe_Nothing : __Maybe_Just(value);
});

// Fetch's `Headers` matches names in any case and joins repeated values.
var _NomiServer_header = F2(function(name, request)
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


// BODIES

function _NomiServer_body(request)
{
	return _Runtime.fromPromise(function() {
		return request.body || (request.body = request.text === undefined ? _NomiServer_read(request.req, request.max) : _NomiServer_given(request.text, request.max));
	});
}

// The body of a request made in a test, too large past `max` bytes.
function _NomiServer_given(text, max)
{
	return new TextEncoder().encode(text).length > max ? Promise.reject(__Server_TooLarge) : Promise.resolve(text);
}

// Reads the body of `req` as UTF-8, rejecting with a `BodyError`: past
// `max` bytes the rest is left unread, and the handler still answers.
function _NomiServer_read(req, max)
{
	if (Number(req.headers.get('content-length')) > max)
	{
		return Promise.reject(__Server_TooLarge);
	}
	if (req.body === null)
	{
		return Promise.resolve('');
	}
	var reader = req.body.getReader();
	var chunks = [];
	var size = 0;
	function next()
	{
		return reader.read().then(function(read) {
			if (read.done)
			{
				return new TextDecoder().decode(Buffer.concat(chunks));
			}
			size += read.value.length;
			if (size > max)
			{
				reader.cancel().catch(function() {});
				throw __Server_TooLarge;
			}
			chunks.push(read.value);
			return next();
		}, function() {
			throw __Server_Disconnected;
		});
	}
	return next();
}
