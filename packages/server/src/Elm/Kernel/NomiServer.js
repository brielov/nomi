/*

import Elm.Kernel.List exposing (fromArray, toArray)
import Elm.Kernel.NomiStdio exposing (log)
import Elm.Kernel.Utils exposing (Tuple0, Tuple2)
import Maybe exposing (Just, Nothing)
import Nomi.Server as Server exposing (AddressInUse, PermissionDenied, Other, TooLarge, Disconnected)

*/


// SERVING

// The program is an ES module, which has no `require`.
var _NomiServer_http = process.getBuiltinModule('node:http');

// How much longer than its drain the program's shutdown may take, for the
// finalizers that run after it.
var _NomiServer_margin = 5000;

// A server on `node:http`. Once it is bound, `onListen` runs with the port
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
					return _Runtime.andThen(config.__$onListen(server.http.address().port), function() {
						return _NomiServer_accept(server, spawn, handle);
					});
				}));
			});
		});
	});
});

// A listening server on `runtime`, with the requests it received that no
// fiber has taken yet, the accept loop's `resume` while it waits for one,
// and whether it stopped accepting. A port Node refuses, as 70000, fails
// as an error does.
function _NomiServer_open(config, runtime)
{
	return _Runtime.async(function(resume) {
		var server = { http: _NomiServer_http.createServer(), runtime: runtime, requests: [], taker: null, stopped: false };
		var max = config.__$maxBodyBytes;
		server.http.on('request', function(req, res) {
			if (server.stopped) return _NomiServer_unavailable(res);
			var request = { req: req, res: res, max: max, body: null, fiber: null, gone: false, server: server };
			// The connection closed before the response was written: the
			// client is gone, and its request is interrupted.
			res.on('close', function() {
				if (res.writableEnded) return;
				request.gone = true;
				request.fiber && runtime.interrupt(request.fiber);
			});
			var taker = server.taker;
			server.taker = null;
			taker ? taker(_Runtime.succeed(request)) : server.requests.push(request);
		});
		server.http.on('error', function(error) {
			resume(_Runtime.fail(_NomiServer_error(error, config)));
		});
		try
		{
			server.http.listen(config.__$port_, config.__$host, function() {
				resume(_Runtime.succeed(server));
			});
		}
		catch (error)
		{
			resume(_Runtime.fail(_NomiServer_error(error, config)));
		}
	});
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
		server.http.close();
		server.requests.splice(0).forEach(function(request) { _NomiServer_unavailable(request.res); });
		return __Utils_Tuple0;
	});
}

function _NomiServer_unavailable(res)
{
	res.writeHead(503, { 'Content-Type': 'text/plain; charset=utf-8', 'Connection': 'close' });
	res.end('the server is stopping');
}

// Closes the connections still open once the drain is over.
function _NomiServer_close(server)
{
	return _Runtime.sync(function() {
		server.http.closeAllConnections();
		return __Utils_Tuple0;
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
			var res = request.res;
			var headers = __List_toArray(response.__$headers);
			for (var i = 0; i < headers.length; i++)
			{
				res.appendHeader(headers[i].a, headers[i].b);
			}
			request.server && request.server.stopped && res.setHeader('Connection', 'close');
			res.statusCode = response.__$status;
			res.end(response.__$body);
			return __Utils_Tuple0;
		});
	});
	return _Runtime.andThen(_Runtime.fork(answered), function(fiber) {
		return _Runtime.andThen(_Runtime.awaitFiber(fiber), function(exit) {
			return exit.$ === _Runtime.ExitKind.DEFECT ? _NomiServer_died(request, exit.defect) : _Runtime.succeed(__Utils_Tuple0);
		});
	});
}

// Answers 500 for a handler that died, or closes the connection when the
// response had begun, and logs one line naming the request, as `Nomi.log`
// does.
function _NomiServer_died(request, defect)
{
	var res = request.res;
	if (res.headersSent)
	{
		res.destroy();
	}
	else if (!res.destroyed)
	{
		res.getHeaderNames().forEach(function(name) { res.removeHeader(name); });
		res.statusCode = 500;
		res.setHeader('Content-Type', 'text/plain; charset=utf-8');
		res.end('internal server error');
	}
	var described = String(defect).replace(/\s*\n\s*/g, ' ');
	return __NomiStdio_log('nomi: request ' + request.req.method + ' ' + request.req.url + ' died: ' + described);
}


// TESTING

// A request with no connection behind it, whose body is `text`, its
// headers read by Node's own rules for repeated ones.
var _NomiServer_request = F4(function(method, target, headers, text)
{
	var raw = [];
	for (var list = headers; list.b; list = list.b) // WHILE_CONS
	{
		raw.push(list.a.a, list.a.b);
	}
	var message = new _NomiServer_http.IncomingMessage(null);
	message._addHeaderLines(raw, raw.length);
	return { req: { method: method, url: target, headers: message.headers }, text: text, body: null };
});

// Answers a request `_NomiServer_request` made as a server does, writing
// to a response that records it, new each time the task runs.
var _NomiServer_answer = F2(function(handle, made)
{
	var received = _Runtime.sync(function() {
		return { req: made.req, res: _NomiServer_recorder(), max: _NomiServer_maxBody, text: made.text, body: null };
	});
	return _Runtime.andThen(received, function(request) {
		return _Runtime.map(_NomiServer_respond(handle, request), function() {
			var res = request.res;
			var headers = res.headers.map(function(header) { return __Utils_Tuple2(header[0], header[1]); });
			return { __$status: res.statusCode, __$headers: __List_fromArray(headers), __$body: res.body };
		});
	});
});

// What answering a request uses of `node:http`'s response: its headers as
// names and values, in the order Node writes them, which is the order
// given with those of one name together, where the first came, a name
// matched in any case. What Node refuses to write, it refuses with Node's
// own error, so a test sees the defect `listen` would.
function _NomiServer_recorder()
{
	var named = function(name) {
		return function(header) { return header[0].toLowerCase() !== name.toLowerCase(); };
	};
	return {
		headers: [],
		statusCode: 200,
		body: '',
		headersSent: false,
		destroyed: false,
		appendHeader: function(name, value) {
			_NomiServer_http.validateHeaderName(name);
			_NomiServer_http.validateHeaderValue(name, value);
			var other = named(name);
			var last = this.headers.findLastIndex(function(header) { return !other(header); });
			this.headers.splice(last < 0 ? this.headers.length : last + 1, 0, [name, value]);
		},
		setHeader: function(name, value) {
			this.removeHeader(name);
			this.appendHeader(name, value);
		},
		removeHeader: function(name) { this.headers = this.headers.filter(named(name)); },
		getHeaderNames: function() { return this.headers.map(function(header) { return header[0].toLowerCase(); }); },
		end: function(body) {
			new _NomiServer_http.ServerResponse(new _NomiServer_http.IncomingMessage(null)).writeHead(this.statusCode);
			this.body = body;
		}
	};
}


// REQUESTS

function _NomiServer_method(request)
{
	return request.req.method;
}

function _NomiServer_path(request)
{
	var target = request.req.url;
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
	var target = request.req.url;
	var start = target.indexOf('?');
	var value = start < 0 ? null : new URLSearchParams(target.slice(start + 1)).get(name);
	return value === null ? __Maybe_Nothing : __Maybe_Just(value);
});

// Node keys headers in lower case, and keeps repeated `Set-Cookie`s apart.
var _NomiServer_header = F2(function(name, request)
{
	var value = request.req.headers[name.toLowerCase()];
	return value === undefined ? __Maybe_Nothing : __Maybe_Just(Array.isArray(value) ? value.join(', ') : value);
});


// BODIES

// The most a request made in a test may hold, in bytes: `defaults`'s.
var _NomiServer_maxBody = 1024 * 1024;

function _NomiServer_body(request)
{
	return _Runtime.fromPromise(function() {
		return request.body || (request.body = request.text === undefined ? _NomiServer_read(request.req, request.max) : _NomiServer_given(request.text, request.max));
	});
}

// The body of a request made in a test, too large past `max` bytes.
function _NomiServer_given(text, max)
{
	return Buffer.byteLength(text) > max ? Promise.reject(__Server_TooLarge) : Promise.resolve(text);
}

// Reads the body of `req` as UTF-8, rejecting with a `BodyError`. A body
// of more than `max` bytes is read on and dropped, so the client still
// gets the response.
function _NomiServer_read(req, max)
{
	return new Promise(function(resolve, reject) {
		var chunks = [];
		var size = 0;
		function tooLarge()
		{
			req.off('data', data);
			req.resume();
			reject(__Server_TooLarge);
		}
		function data(chunk)
		{
			size += chunk.length;
			size > max ? tooLarge() : chunks.push(chunk);
		}
		if (Number(req.headers['content-length']) > max)
		{
			return tooLarge();
		}
		req.on('data', data);
		req.on('end', function() { resolve(Buffer.concat(chunks).toString('utf8')); });
		req.on('error', function() { reject(__Server_Disconnected); });
		req.on('close', function() { req.complete || reject(__Server_Disconnected); });
	});
}
