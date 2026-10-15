/*

import Elm.Kernel.List exposing (fromArray, toArray)
import Elm.Kernel.Scheduler exposing (runtime)
import Elm.Kernel.Utils exposing (Tuple0)
import Maybe exposing (Just, Nothing)
import Nomi.Server as Server exposing (AddressInUse, PermissionDenied, OtherError, TooLarge, Disconnected)

*/


// SERVING

// The program is an ES module, which has no `require`.
var _NomiServer_http = process.getBuiltinModule('node:http');

// How much longer than its drain the program's shutdown may take, for the
// finalizers that run after it.
var _NomiServer_margin = 5000;

// A server on `node:http`. Its requests are the fibers of a draining
// scope, so they outlive the accept loop: interrupting `listen` closes the
// accept loop's scope, which stops the server accepting connections, then
// the draining scope gives the requests in flight the drain to finish, and
// finally closes every connection left.
var _NomiServer_listen = F3(function(config, handle, net)
{
	var drain = config.__$drainMilliseconds;
	var allowed = _Runtime.sync(function() {
		_Runtime.allowShutdown(__Scheduler_runtime, drain + _NomiServer_margin);
	});
	return _Runtime.andThen(allowed, function() {
		return _Runtime.draining(drain, function(spawn) {
			var opened = _Runtime.acquireRelease(_NomiServer_open(config), _NomiServer_close);
			return _Runtime.andThen(opened, function(server) {
				var accepting = _Runtime.addFinalizer(function() { return _NomiServer_stop(server); });
				return _Runtime.scoped(_Runtime.andThen(accepting, function() {
					return _NomiServer_accept(server, spawn, handle);
				}));
			});
		});
	});
});

// A listening server, with the requests it received that no fiber has
// taken yet, and the accept loop's `resume` while it waits for one.
function _NomiServer_open(config)
{
	return _Runtime.async(function(resume) {
		var server = { http: _NomiServer_http.createServer(), requests: [], taker: null };
		server.http.on('request', function(req, res) {
			var request = { req: req, res: res, body: null };
			var taker = server.taker;
			server.taker = null;
			taker ? taker(_Runtime.succeed(request)) : server.requests.push(request);
		});
		server.http.on('error', function(error) {
			resume(_Runtime.fail(_NomiServer_error(error)));
		});
		server.http.listen(config.__$port_, config.__$host, function() {
			resume(_Runtime.succeed(server));
		});
	});
}

// Takes each request as it comes and spawns its response in the draining
// scope.
function _NomiServer_accept(server, spawn, handle)
{
	var next = _Runtime.async(function(resume) {
		if (server.requests.length > 0)
		{
			resume(_Runtime.succeed(server.requests.shift()));
			return;
		}
		server.taker = resume;
		return function() { server.taker = null; };
	});
	var loop = _Runtime.andThen(next, function(request) {
		return _Runtime.andThen(spawn(_NomiServer_respond(handle, request)), function() { return loop; });
	});
	return loop;
}

// Refuses new connections and closes the idle ones.
function _NomiServer_stop(server)
{
	return _Runtime.sync(function() {
		server.http.close();
		return __Utils_Tuple0;
	});
}

// Closes the connections still open once the drain is over.
function _NomiServer_close(server)
{
	return _Runtime.sync(function() {
		server.http.closeAllConnections();
		return __Utils_Tuple0;
	});
}

function _NomiServer_error(error)
{
	switch (error.code)
	{
		case 'EADDRINUSE':
			return __Server_AddressInUse;
		case 'EACCES':
			return __Server_PermissionDenied;
		default:
			return __Server_OtherError(error.message);
	}
}

// Answers `request` with the response the handler gives.
function _NomiServer_respond(handle, request)
{
	return _Runtime.andThen(handle(request), function(response) {
		return _Runtime.sync(function() {
			var res = request.res;
			var headers = __List_toArray(response.__$headers);
			for (var i = 0; i < headers.length; i++)
			{
				res.setHeader(headers[i].a, headers[i].b);
			}
			res.statusCode = response.__$status;
			res.end(response.__$body);
			return __Utils_Tuple0;
		});
	});
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

// The most a request's body may hold, in bytes.
var _NomiServer_maxBody = 1024 * 1024;

function _NomiServer_body(request)
{
	return _Runtime.fromPromise(function() {
		return request.body || (request.body = _NomiServer_read(request.req));
	});
}

// Reads the body of `req` as UTF-8, rejecting with a `BodyError`. A body
// too large is read on and dropped, so the client still gets the response.
function _NomiServer_read(req)
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
			size > _NomiServer_maxBody ? tooLarge() : chunks.push(chunk);
		}
		if (Number(req.headers['content-length']) > _NomiServer_maxBody)
		{
			return tooLarge();
		}
		req.on('data', data);
		req.on('end', function() { resolve(Buffer.concat(chunks).toString('utf8')); });
		req.on('error', function() { reject(__Server_Disconnected); });
		req.on('close', function() { req.complete || reject(__Server_Disconnected); });
	});
}
