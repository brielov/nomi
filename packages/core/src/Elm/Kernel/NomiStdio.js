/*

import Elm.Kernel.NomiStream exposing (reader, give, fail, error, bytes, view, writer, write, capture, captured, held)
import Elm.Kernel.Utils exposing (Tuple0)

*/


// THE STANDARD STREAMS

// The process's standard input, read only while a read waits that its
// buffer can't answer, so input nobody asked for stays where it is, and an
// open input nobody reads doesn't keep the process alive.
var _NomiStdio_listening = false;
var _NomiStdio_stdin = __NomiStream_reader(true, function(reader) {
	var stream = process.stdin;
	if (reader.waiting.length === 0 || reader.end !== null)
	{
		if (!_NomiStdio_listening) return;
		stream.pause();
		stream.unref && stream.unref();
		return;
	}
	if (!_NomiStdio_listening)
	{
		_NomiStdio_listening = true;
		stream.on('data', function(chunk) { __NomiStream_give(reader, __NomiStream_bytes(chunk)); });
		stream.on('end', function() { __NomiStream_give(reader, null); });
		stream.on('error', function(error) { __NomiStream_fail(reader, { error: __NomiStream_error(error) }); });
	}
	stream.ref && stream.ref();
	stream.resume();
});
_NomiStdio_stdin.interactive = !!process.stdin.isTTY;

// A writer to the process's `stream`, each write waiting until the stream
// has taken its bytes, which a pipe nobody reads holds back. Output a
// reader closed, as a pipe into `head`, is dropped, so writing to the
// standard output and error never fails.
function _NomiStdio_writer(stream)
{
	return __NomiStream_writer({
		write: function(bytes) {
			return _Runtime.async(function(resume) {
				stream.listenerCount('error') > 0 || stream.on('error', function() {});
				stream.write(__NomiStream_view(bytes), function() { resume(_Runtime.succeed(__Utils_Tuple0)); });
			});
		},
		close: _Runtime.succeed(__Utils_Tuple0),
		terminal: !!stream.isTTY,
		size: function() {
			return stream.isTTY ? { columns: stream.columns, rows: stream.rows } : null;
		}
	});
}

var _NomiStdio_stdout = _NomiStdio_writer(process.stdout);
var _NomiStdio_stderr = _NomiStdio_writer(process.stderr);


// TEXT

var _NomiStdio_encoder = new TextEncoder();

var _NomiStdio_print = F2(function(text, writer)
{
	return A2(__NomiStream_write, __NomiStream_bytes(_NomiStdio_encoder.encode(text)), writer);
});

// The writer `Nomi.log` writes to on a runtime other than the process's,
// which is a test's captured stderr on that test's runtime.
var _NomiStdio_logs = new WeakMap();

// A line to the stderr of whatever runs the task: the process's, or the
// test's. A log line can't fail, so a stderr that fails drops it.
function _NomiStdio_log(text)
{
	return _Runtime.withRuntime(function(runtime) {
		var line = A2(_NomiStdio_print, text + '\n', _NomiStdio_logs.get(runtime) || _NomiStdio_stderr);
		return _Runtime.onError(line, function() { return _Runtime.succeed(__Utils_Tuple0); });
	});
}


// FAKES

// A reader of `bytes`, which then ends; it keeps them, so a test can
// script it anew.
function _NomiStdio_script(bytes)
{
	var reader = __NomiStream_held(bytes);
	reader.script = bytes;
	return reader;
}

var _NomiStdio_capture = __NomiStream_capture;
var _NomiStdio_captured = __NomiStream_captured;
