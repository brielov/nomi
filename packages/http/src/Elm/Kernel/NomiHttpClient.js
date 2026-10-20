/*

import Elm.Kernel.List exposing (fromArray, toArray)
import Elm.Kernel.NomiHttpServer exposing (loopbackAnswer, loopbackResponse)
import Elm.Kernel.NomiNet exposing (allows, key)
import Elm.Kernel.NomiStream exposing (fromWeb, held, view)
import Elm.Kernel.Utils exposing (Tuple2)
import Maybe exposing (Just, Nothing, isJust)
import Nomi.Http.Client as Client exposing (BadUrl, NotAllowed, Unreachable, Timeout, TooManyRedirects)

*/


// SENDING

// How many redirects a request follows.
var _NomiHttpClient_mostRedirects = 10;

// Sends `r`, the request's fields, on `net`: each hop a request to a host
// the network allows, on it or on its loopback, following redirects here
// rather than in `fetch`, so the capability judges each. The response is
// the fiber's that sent it: its body, unread once the fiber ends, is
// dropped.
var _NomiHttpClient_send = F2(function(r, net)
{
	var headers = __List_toArray(r.__$headers).map(function(pair) { return [pair.a, pair.b]; });
	var body = __Maybe_isJust(r.__$body) ? __NomiStream_view(r.__$body.a) : null;
	// The timeout is a deadline on the program's clock, which each hop's
	// wait for its response races, rather than a race around them all:
	// a handler on a loopback answers on a fiber of the sender's.
	var sent = _Runtime.andThen(_Runtime.now, function(start) {
		var deadline = __Maybe_isJust(r.__$timeout) ? start + r.__$timeout.a : null;
		return _NomiHttpClient_hop(net, r.__$method, r.__$url, headers, body, 0, deadline);
	});
	// The fiber's scope drops the body once the fiber ends, a finalizer
	// added before the request, which stays interruptible, is sent.
	var given = { body: null };
	var dropping = _Runtime.addFinalizer(function() {
		return _Runtime.sync(function() {
			given.body && given.body.cancel && given.body.cancel();
			return 0;
		});
	});
	var owned = _Runtime.andThen(dropping, function() {
		return _Runtime.map(sent, function(response) {
			given.body = response.body;
			return response;
		});
	});
	return _Runtime.map(owned, function(response) {
		var listed = [];
		response.answer.headers.forEach(function(value, name) {
			name === 'set-cookie' || listed.push([name, value]);
		});
		response.answer.headers.getSetCookie().forEach(function(value) { listed.push(['set-cookie', value]); });
		listed.sort(function(a, b) { return a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0; });
		return {
			__$status: response.answer.status,
			__$headers: __List_fromArray(listed.map(function(pair) { return __Utils_Tuple2(pair[0], pair[1]); })),
			__$body: response.body
		};
	});
});

// One request, and the hops its redirects lead to.
function _NomiHttpClient_hop(net, method, address, headers, body, hops, deadline)
{
	var url;
	try
	{
		url = new URL(address);
	}
	catch (e)
	{
		return _Runtime.fail(__Client_BadUrl(address));
	}
	if (url.protocol !== 'http:' && url.protocol !== 'https:')
	{
		return _Runtime.fail(__Client_BadUrl(address));
	}
	if (!__NomiNet_allows(net, url.hostname))
	{
		return _Runtime.fail(__Client_NotAllowed(url.hostname));
	}
	var answered = _Runtime.andThen(_Runtime.now, function(now) {
		var left = deadline === null ? null : Math.max(0, deadline - now);
		return net.loopback !== null
			? _NomiHttpClient_loopback(net.loopback, method, url, headers, body, left)
			: _Runtime.onError(
				_Runtime.fromPromise(function(signal) {
					var signals = left === null ? signal : AbortSignal.any([signal, AbortSignal.timeout(left)]);
					return fetch(url, { method: method, headers: headers, body: body, redirect: 'manual', signal: signals });
				}),
				function(error) {
					return _Runtime.fail(error && error.name === 'TimeoutError'
						? __Client_Timeout
						: __Client_Unreachable(url.host + ': ' + (error && error.message || error)));
				}
			);
	});
	return _Runtime.andThen(answered, function(answer) {
		var location = answer.headers.get('location');
		if (!_NomiHttpClient_redirects[answer.status] || location === null)
		{
			var reader = answer.body === null ? __NomiStream_held(new DataView(new ArrayBuffer(0))) : __NomiStream_fromWeb(answer.body);
			return _Runtime.succeed({ answer: answer, body: reader });
		}
		answer.body && answer.body.cancel().catch(function() {});
		if (hops >= _NomiHttpClient_mostRedirects)
		{
			return _Runtime.fail(__Client_TooManyRedirects);
		}
		var next;
		try
		{
			next = new URL(location, url);
		}
		catch (e)
		{
			return _Runtime.fail(__Client_BadUrl(location));
		}
		// 303, and 301 or 302 after a POST, go on as a GET with no body.
		var asGet = answer.status === 303 && method !== 'HEAD' || (answer.status === 301 || answer.status === 302) && method === 'POST';
		var carried = headers.filter(function(pair) {
			var name = pair[0].toLowerCase();
			return !(next.host !== url.host && (name === 'authorization' || name === 'cookie'))
				&& !(asGet && (name === 'content-type' || name === 'content-length'));
		});
		return _NomiHttpClient_hop(net, asGet ? 'GET' : method, next.href, carried, asGet ? null : body, hops + 1, deadline);
	});
}

var _NomiHttpClient_redirects = { 301: true, 302: true, 303: true, 307: true, 308: true };

// A request to a loopback's server, answered by its handler on a fiber of
// the sender's, as a connection would: its response as `fetch` gives one,
// waited for until `left` milliseconds passed, when there is a timeout.
function _NomiHttpClient_loopback(loopback, method, url, headers, body, left)
{
	var port = url.port !== '' ? Number(url.port) : url.protocol === 'https:' ? 443 : 80;
	var server = loopback.servers.get(__NomiNet_key(url.hostname, port));
	if (server === undefined)
	{
		return _Runtime.fail(__Client_Unreachable(url.host + ': connection refused'));
	}
	var given = new Headers();
	headers.forEach(function(pair) { given.append(pair[0], pair[1]); });
	return _Runtime.andThen(__NomiHttpServer_loopbackAnswer(server, method, url.pathname + url.search, given, body), function(request) {
		var response = __NomiHttpServer_loopbackResponse(request);
		// A request given up is a client going away: its handler is
		// interrupted, as a socket's would be.
		var givenUp = _Runtime.andThen(_Runtime.sleep(left), function() {
			return _Runtime.andThen(_Runtime.interrupt(request.exchange), function() {
				return _Runtime.fail(__Client_Timeout);
			});
		});
		return left === null ? response : _Runtime.race([response, givenUp]);
	});
}
