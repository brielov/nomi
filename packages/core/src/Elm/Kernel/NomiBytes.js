/*

import Elm.Kernel.List exposing (toArray)
import Maybe exposing (Just, Nothing)

*/


// Bytes are elm/bytes' `DataView`s. A slice shares its buffer, as
// elm/bytes' own `Decode.bytes` does: nothing writes to `Bytes` once they
// are made.

function _NomiBytes_view(bytes)
{
	return new Uint8Array(bytes.buffer, bytes.byteOffset, bytes.byteLength);
}

function _NomiBytes_bytes(array)
{
	return new DataView(array.buffer, array.byteOffset, array.byteLength);
}


// MEASURING AND CUTTING

function _NomiBytes_length(bytes)
{
	return bytes.byteLength;
}

// `subarray` counts a negative index from the end and stops an index at
// either end, as `String.slice` does.
var _NomiBytes_slice = F3(function(start, end, bytes)
{
	return _NomiBytes_bytes(_NomiBytes_view(bytes).subarray(start, end));
});

var _NomiBytes_append = F2(function(a, b)
{
	return _NomiBytes_join([a, b]);
});

function _NomiBytes_concat(list)
{
	return _NomiBytes_join(__List_toArray(list));
}

function _NomiBytes_join(all)
{
	var size = 0;
	for (var i = 0; i < all.length; i++)
	{
		size += all[i].byteLength;
	}
	var joined = new Uint8Array(size);
	for (var i = 0, at = 0; i < all.length; at += all[i].byteLength, i++)
	{
		joined.set(_NomiBytes_view(all[i]), at);
	}
	return _NomiBytes_bytes(joined);
}


// COMPARING AND SEARCHING

var _NomiBytes_equal = F2(function(a, b)
{
	if (a.byteLength !== b.byteLength)
	{
		return false;
	}
	var x = _NomiBytes_view(a);
	var y = _NomiBytes_view(b);
	for (var i = 0; i < x.length; i++)
	{
		if (x[i] !== y[i])
		{
			return false;
		}
	}
	return true;
});

// Bun's `Buffer` searches natively; these views copy nothing.
var _NomiBytes_indexOf = F2(function(needle, haystack)
{
	var at = Buffer.from(haystack.buffer, haystack.byteOffset, haystack.byteLength)
		.indexOf(Buffer.from(needle.buffer, needle.byteOffset, needle.byteLength));
	return at < 0 ? __Maybe_Nothing : __Maybe_Just(at);
});


// TEXT

var _NomiBytes_encoder = new TextEncoder();

// Refuses what isn't UTF-8 rather than replacing it, and keeps a byte
// order mark as the U+FEFF it is.
var _NomiBytes_decoder = new TextDecoder('utf-8', { fatal: true, ignoreBOM: true });

function _NomiBytes_fromString(string)
{
	return _NomiBytes_bytes(_NomiBytes_encoder.encode(string));
}

function _NomiBytes_toString(bytes)
{
	try
	{
		return __Maybe_Just(_NomiBytes_decoder.decode(bytes));
	}
	catch (e)
	{
		return __Maybe_Nothing;
	}
}


// BASE64 AND HEX

function _NomiBytes_toBase64(bytes)
{
	return _NomiBytes_view(bytes).toBase64();
}

function _NomiBytes_toBase64Url(bytes)
{
	return _NomiBytes_view(bytes).toBase64({ alphabet: 'base64url', omitPadding: true });
}

// Either alphabet, padded or not. `fromBase64` skips whitespace, so the
// characters are checked first; padding must fill the last group, and
// 'strict' refuses a last character whose unused bits aren't zero.
function _NomiBytes_fromBase64(string)
{
	var url = /[-_]/.test(string);
	var valid = url ? /^[A-Za-z0-9_-]*={0,2}$/ : /^[A-Za-z0-9+\/]*={0,2}$/;
	if (!valid.test(string) || (string.endsWith('=') && string.length % 4 !== 0))
	{
		return __Maybe_Nothing;
	}
	var padded = string + '='.repeat((4 - string.length % 4) % 4);
	try
	{
		var options = { alphabet: url ? 'base64url' : 'base64', lastChunkHandling: 'strict' };
		return __Maybe_Just(_NomiBytes_bytes(Uint8Array.fromBase64(padded, options)));
	}
	catch (e)
	{
		return __Maybe_Nothing;
	}
}

function _NomiBytes_toHex(bytes)
{
	return _NomiBytes_view(bytes).toHex();
}

function _NomiBytes_fromHex(string)
{
	try
	{
		return __Maybe_Just(_NomiBytes_bytes(Uint8Array.fromHex(string)));
	}
	catch (e)
	{
		return __Maybe_Nothing;
	}
}
