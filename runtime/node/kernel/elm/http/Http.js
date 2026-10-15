/*

import Dict exposing (empty, insert)
import Elm.Kernel.Scheduler exposing (binding, rawSpawn)
import Elm.Kernel.Utils exposing (Tuple2)
import Http exposing (BadUrl_, Timeout_, NetworkError_, BadStatus_, GoodStatus_, Receiving)
import Maybe exposing (Just, Nothing, isJust)
import Platform exposing (sendToSelf)

*/


// elm/http's requests on `fetch`, giving the responses upstream's
// XMLHttpRequest gives: a URL or method `fetch` refuses is `BadUrl_`, a
// rejection `NetworkError_`, and the request's timeout covers the whole
// response, on the host's timer as the network is the host's. Killing the
// process aborts the request, which then answers nothing, as upstream's
// aborted XMLHttpRequest fires none of the events it listens for. `fetch`
// reports no upload progress, so a tracker hears only `Receiving`.


// SEND REQUEST

var _Http_toTask = F3(function(router, toTask, request)
{
	return __Scheduler_binding(function(callback)
	{
		var controller = new AbortController();
		var timer;
		function done(response) {
			clearTimeout(timer);
			controller.signal.aborted || callback(toTask(request.__$expect.__toValue(response)));
		}

		try {
			new Request(request.__$url, { method: request.__$method });
		} catch (e) {
			return done(__Http_BadUrl_(request.__$url));
		}

		var init = _Http_configureRequest(request, controller.signal);
		if (request.__$timeout.a)
		{
			timer = setTimeout(function() { done(__Http_Timeout_); controller.abort(); }, request.__$timeout.a);
		}
		fetch(request.__$url, init)
			.then(function(response) {
				var tracker = __Maybe_isJust(request.__$tracker) && request.__$tracker.a;
				return _Http_toResponse(request.__$expect, response, tracker && _Http_track(router, controller.signal, tracker, response));
			})
			.then(done, function() { done(__Http_NetworkError_); });

		return function() { clearTimeout(timer); controller.abort(); };
	});
});


// CONFIGURE

// XMLHttpRequest sends no body with GET or HEAD, where `fetch` refuses one.
function _Http_configureRequest(request, signal)
{
	var headers = new Headers();
	for (var list = request.__$headers; list.b; list = list.b) // WHILE_CONS
	{
		headers.append(list.a.a, list.a.b);
	}
	request.__$body.a && headers.append('Content-Type', request.__$body.a);
	return {
		method: request.__$method,
		headers: headers,
		body: /^(GET|HEAD)$/i.test(request.__$method) ? undefined : request.__$body.b,
		signal: signal
	};
}


// RESPONSES

// The response once its body has arrived, each chunk told to `progress`.
async function _Http_toResponse(expect, response, progress)
{
	var chunks = [];
	var received = 0;
	if (response.body)
	{
		for await (var chunk of response.body)
		{
			chunks.push(chunk);
			received += chunk.length;
			progress && progress(received, false);
		}
	}
	progress && progress(received, true);
	var bytes = new Uint8Array(received);
	for (var i = 0, at = 0; i < chunks.length; at += chunks[i++].length)
	{
		bytes.set(chunks[i], at);
	}
	return A2(
		200 <= response.status && response.status < 300 ? __Http_GoodStatus_ : __Http_BadStatus_,
		_Http_toMetadata(response),
		expect.__toBody(expect.__type === 'arraybuffer' ? bytes.buffer : new TextDecoder().decode(bytes))
	);
}


// METADATA

function _Http_toMetadata(response)
{
	return {
		__$url: response.url,
		__$statusCode: response.status,
		__$statusText: response.statusText,
		__$headers: _Http_parseHeaders(response.headers)
	};
}


// HEADERS

// `fetch` gives the names in lower case and joins a name's values with
// ", ", as XMLHttpRequest's `getAllResponseHeaders` does, which leaves out
// `Set-Cookie`.
function _Http_parseHeaders(fetched)
{
	var headers = __Dict_empty;
	fetched.forEach(function(value, key)
	{
		if (key !== 'set-cookie' && key !== 'set-cookie2')
		{
			headers = A3(__Dict_insert, key, value, headers);
		}
	});
	return headers;
}


// EXPECT

var _Http_expect = F3(function(type, toBody, toValue)
{
	return {
		$: 0,
		__type: type,
		__toBody: toBody,
		__toValue: toValue
	};
});

var _Http_mapExpect = F2(function(func, expect)
{
	return {
		$: 0,
		__type: expect.__type,
		__toBody: expect.__toBody,
		__toValue: function(x) { return func(expect.__toValue(x)); }
	};
});

function _Http_toDataView(arrayBuffer)
{
	return new DataView(arrayBuffer);
}


// BODY and PARTS

var _Http_emptyBody = { $: 0 };
var _Http_pair = F2(function(a, b) { return { $: 0, a: a, b: b }; });

function _Http_toFormData(parts)
{
	for (var formData = new FormData(); parts.b; parts = parts.b) // WHILE_CONS
	{
		var part = parts.a;
		formData.append(part.a, part.b);
	}
	return formData;
}

var _Http_bytesToBlob = F2(function(mime, bytes)
{
	return new Blob([bytes], { type: mime });
});


// PROGRESS

// Reports what has been received as XMLHttpRequest's `progress` events
// do: at most every 50 ms while the body arrives, and once at its end. The
// size is the `Content-Length`, unless the body is encoded, as `fetch`
// counts its decoded bytes.
function _Http_track(router, signal, tracker, response)
{
	var length = response.headers.get('content-encoding') ? null : response.headers.get('content-length');
	var size = length === null ? __Maybe_Nothing : __Maybe_Just(Number(length));
	var last = 0;
	return function(received, end)
	{
		var now = Date.now();
		if (signal.aborted || !end && now - last < 50) { return; }
		last = now;
		__Scheduler_rawSpawn(A2(__Platform_sendToSelf, router, __Utils_Tuple2(tracker, __Http_Receiving({
			__$received: received,
			__$size: size
		}))));
	};
}
