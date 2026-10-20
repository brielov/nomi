/*

import Elm.Kernel.NomiStream exposing (bytes, view)
import Maybe exposing (Just, Nothing)
import Nomi.Stream as Stream exposing (Invalid)

*/


var _NomiCompress_zlib = process.getBuiltinModule('node:zlib');

// The most bytes `decompress` gives, past which it gives nothing, as a
// small input can unpack to far more than memory holds.
var _NomiCompress_most = 64 * 1024 * 1024;

// Each format's functions in node:zlib, whole and streamed.
var _NomiCompress_formats = {
	gzip: ['gzipSync', 'gunzipSync', 'createGzip', 'createGunzip'],
	deflate: ['deflateSync', 'inflateSync', 'createDeflate', 'createInflate'],
	brotli: ['brotliCompressSync', 'brotliDecompressSync', 'createBrotliCompress', 'createBrotliDecompress'],
	zstd: ['zstdCompressSync', 'zstdDecompressSync', 'createZstdCompress', 'createZstdDecompress']
};


// WHOLE

var _NomiCompress_compress = F2(function(format, data)
{
	return __NomiStream_bytes(_NomiCompress_zlib[_NomiCompress_formats[format][0]](__NomiStream_view(data)));
});

var _NomiCompress_decompress = F2(function(format, data)
{
	try
	{
		var whole = _NomiCompress_zlib[_NomiCompress_formats[format][1]](__NomiStream_view(data), { maxOutputLength: _NomiCompress_most });
		return __Maybe_Just(__NomiStream_bytes(whole));
	}
	catch (e)
	{
		return __Maybe_Nothing;
	}
});


// STREAMED

// A codec: node:zlib's stream, compressing or decompressing, the output it
// gave that no push took yet, how it failed, if it did, and whether it
// was ended.
var _NomiCompress_codec = F2(function(compressing, format)
{
	return _Runtime.sync(function() {
		var stream = _NomiCompress_zlib[_NomiCompress_formats[format][compressing ? 2 : 3]]();
		var codec = { stream: stream, out: [], failed: null, ended: false };
		stream.on('data', function(chunk) { codec.out.push(chunk); });
		stream.on('error', function(error) { codec.failed = codec.failed || error; });
		return codec;
	});
});

// What the codec gave so far, taken.
function _NomiCompress_taken(codec)
{
	return __NomiStream_bytes(Buffer.concat(codec.out.splice(0)));
}

function _NomiCompress_failure(error)
{
	return _Runtime.fail(__Stream_Invalid(String(error && error.message || error)));
}

// Gives the codec the bytes, and succeeds with what it gave for them, which
// may be nothing yet.
var _NomiCompress_push = F2(function(data, codec)
{
	var pushed = _Runtime.fromPromise(function() {
		return new Promise(function(resolve, reject) {
			if (codec.failed) return reject(codec.failed);
			// A stream that fails may never call the write's callback.
			var failed = function(error) { reject(error); };
			codec.stream.once('error', failed);
			codec.stream.write(__NomiStream_view(data), function(error) {
				codec.stream.off('error', failed);
				error || codec.failed ? reject(error || codec.failed) : resolve(_NomiCompress_taken(codec));
			});
		});
	});
	return _Runtime.onError(pushed, _NomiCompress_failure);
});

// Ends the codec's input, and succeeds with the rest of its output, or
// `Nothing` once it gave all of it.
function _NomiCompress_finish(codec)
{
	if (codec.ended)
	{
		return _Runtime.succeed(__Maybe_Nothing);
	}
	codec.ended = true;
	var finished = _Runtime.fromPromise(function() {
		return new Promise(function(resolve, reject) {
			if (codec.failed) return reject(codec.failed);
			codec.stream.on('end', function() {
				codec.failed ? reject(codec.failed) : resolve(_NomiCompress_taken(codec));
			});
			codec.stream.on('error', reject);
			codec.stream.end();
		});
	});
	return _Runtime.onError(
		_Runtime.map(finished, function(rest) { return rest.byteLength > 0 ? __Maybe_Just(rest) : __Maybe_Nothing; }),
		_NomiCompress_failure
	);
}
