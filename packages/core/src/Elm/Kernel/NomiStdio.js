/*

import Elm.Kernel.Utils exposing (Tuple0)
import Maybe exposing (Just, Nothing)

*/


// READERS

// A reader of lines: the text past the last whole line, the lines no one
// has taken yet, whether the input ended, and the fibers waiting for a
// line, oldest first. `flow(reader)` asks its source for input while a
// fiber waits, and to stop once none does.
function _NomiStdio_reader(flow)
{
	return { partial: '', lines: [], ended: false, waiting: [], flow: flow };
}

// Takes `text` in, or the end of the input when it is null, and hands the
// lines it completes to the fibers waiting. A line ends at `\n`, and
// `\r\n`; the text after the last one is a line once the input ends.
function _NomiStdio_take(reader, text)
{
	if (text === null)
	{
		reader.partial === '' || reader.lines.push(reader.partial);
		reader.partial = '';
		reader.ended = true;
	}
	else
	{
		var lines = (reader.partial + text).split('\n');
		reader.partial = lines.pop();
		for (var i = 0; i < lines.length; i++)
		{
			reader.lines.push(lines[i].endsWith('\r') ? lines[i].slice(0, -1) : lines[i]);
		}
	}
	while (reader.waiting.length > 0 && (reader.lines.length > 0 || reader.ended))
	{
		reader.waiting.shift()(_NomiStdio_next(reader));
	}
	reader.flow(reader);
}

// The next line, or `Nothing` once the input ended.
function _NomiStdio_next(reader)
{
	return _Runtime.succeed(reader.lines.length > 0 ? __Maybe_Just(reader.lines.shift()) : __Maybe_Nothing);
}

// A line is handed only to a fiber still waiting, so an interrupted
// `line` loses none.
function _NomiStdio_line(reader)
{
	return _Runtime.async(function(resume) {
		if (reader.lines.length > 0 || reader.ended)
		{
			resume(_NomiStdio_next(reader));
			return;
		}
		reader.waiting.push(resume);
		reader.flow(reader);
		return function() {
			reader.waiting.splice(reader.waiting.indexOf(resume), 1);
			reader.flow(reader);
		};
	});
}

// The process's standard input, read only while a fiber waits for a line,
// so input nobody asked for stays where it is, and an open input nobody
// reads doesn't keep the process alive.
var _NomiStdio_listening = false;
var _NomiStdio_stdin = _NomiStdio_reader(function(reader) {
	var stream = process.stdin;
	if (reader.waiting.length === 0)
	{
		if (!_NomiStdio_listening) return;
		stream.pause();
		stream.unref && stream.unref();
		return;
	}
	if (!_NomiStdio_listening)
	{
		_NomiStdio_listening = true;
		stream.setEncoding('utf8');
		stream.on('data', function(text) { _NomiStdio_take(reader, text); });
		stream.on('end', function() { _NomiStdio_take(reader, null); });
		// An input that fails has ended.
		stream.on('error', function() { _NomiStdio_take(reader, null); });
	}
	stream.ref && stream.ref();
	stream.resume();
});

// A reader of `text`, which ends after it.
function _NomiStdio_script(text)
{
	var reader = _NomiStdio_reader(function() {});
	_NomiStdio_take(reader, text);
	_NomiStdio_take(reader, null);
	return reader;
}


// WRITERS

// A writer to `stream`, or with none, a capture of what is printed.
function _NomiStdio_writer(stream)
{
	return { stream: stream, captured: '' };
}

var _NomiStdio_stdout = _NomiStdio_writer(process.stdout);
var _NomiStdio_stderr = _NomiStdio_writer(process.stderr);

function _NomiStdio_capture()
{
	return _NomiStdio_writer(null);
}

function _NomiStdio_captured(writer)
{
	return writer.captured;
}

// Waits until the stream has taken the text. Output a reader closed, as a
// pipe into `head`, is dropped: printing can't fail.
// The writer `Nomi.log` writes to on a runtime other than the process's
// stderr, which is a test's captured stderr on that test's runtime.
var _NomiStdio_logs = new WeakMap();

// A line to the stderr of whatever runs the task: the process's, or the
// test's.
function _NomiStdio_log(text)
{
	return _Runtime.withRuntime(function(runtime) {
		return A2(_NomiStdio_print, text + '\n', _NomiStdio_logs.get(runtime) || _NomiStdio_stderr);
	});
}

var _NomiStdio_print = F2(function(text, writer)
{
	var stream = writer.stream;
	if (stream === null)
	{
		return _Runtime.sync(function() {
			writer.captured += text;
			return __Utils_Tuple0;
		});
	}
	return _Runtime.async(function(resume) {
		stream.listenerCount('error') > 0 || stream.on('error', function() {});
		stream.write(text, function() { resume(_Runtime.succeed(__Utils_Tuple0)); });
	});
});
