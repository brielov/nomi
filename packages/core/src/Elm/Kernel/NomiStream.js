/*

import Elm.Kernel.Utils exposing (Tuple0)
import Maybe exposing (Just, Nothing, isJust)
import Nomi.Stream as Stream exposing (Closed, Invalid, ConnectionReset, PermissionDenied, TimedOut, Other)

*/


// BYTES

// Bytes are elm/bytes' `DataView`s; a host gives `Uint8Array`s.

function _NomiStream_view(bytes)
{
	return new Uint8Array(bytes.buffer, bytes.byteOffset, bytes.byteLength);
}

function _NomiStream_bytes(array)
{
	return new DataView(array.buffer, array.byteOffset, array.byteLength);
}

// A host's error as a `Stream.Error`, by its code.
function _NomiStream_error(error)
{
	switch (error && error.code)
	{
		case 'EPIPE':
		case 'ERR_STREAM_DESTROYED':
			return __Stream_Closed;
		case 'ECONNRESET':
			return __Stream_ConnectionReset;
		case 'EACCES':
		case 'EPERM':
			return __Stream_PermissionDenied;
		case 'ETIMEDOUT':
			return __Stream_TimedOut;
		default:
			return __Stream_Other(String(error && error.message || error));
	}
}


// READERS

// A reader: the chunks its source gave that no read took yet, oldest
// first; how its source ended, once it did (`{ error }`, `{ defect }`, or
// neither at the end of the input); the reads waiting, oldest first, each
// for a chunk or a line; how many buffered bytes hold no newline, so a
// line isn't searched for twice; whether a read waits for the host rather
// than for another fiber; whether it is a terminal's keyboard; and
// `demand(reader)`, which asks its source for more while a read waits
// that the buffer can't answer, and lets it rest otherwise.
//
// A source only ever adds to the buffer, and a read takes from it only as
// it is handed over, which happens before any interruption: an
// interrupted read loses nothing, and reads are answered in the order
// they came.
function _NomiStream_reader(host, demand)
{
	return { chunks: [], end: null, waiting: [], scanned: 0, host: host, interactive: false, demand: demand };
}

function _NomiStream_read(reader)
{
	return _NomiStream_take(reader, false);
}

function _NomiStream_take(reader, line)
{
	var wait = reader.host ? _Runtime.waitHost : _Runtime.wait;
	return wait(function(resume) {
		var waiter = { resume: resume, line: line };
		reader.waiting.push(waiter);
		_NomiStream_deliver(reader);
		return function() {
			_NomiStream_remove(reader.waiting, waiter);
			_NomiStream_deliver(reader);
		};
	});
}

function _NomiStream_remove(waiting, waiter)
{
	var at = waiting.indexOf(waiter);
	at < 0 || waiting.splice(at, 1);
}

// Answers the reads the buffer can, in order, then tells the source
// whether more is wanted.
function _NomiStream_deliver(reader)
{
	while (reader.waiting.length > 0)
	{
		var next = reader.waiting[0].line ? _NomiStream_nextLine(reader) : _NomiStream_nextChunk(reader);
		if (next === null)
		{
			break;
		}
		reader.waiting.shift().resume(next);
	}
	reader.demand(reader);
}

// Whether a read waits that the buffer can't answer.
function _NomiStream_wanting(reader)
{
	return reader.waiting.length > 0 && reader.end === null;
}

function _NomiStream_nextChunk(reader)
{
	if (reader.chunks.length > 0)
	{
		reader.scanned = 0;
		return _Runtime.succeed(__Maybe_Just(reader.chunks.shift()));
	}
	return reader.end && _NomiStream_ended(reader.end);
}

function _NomiStream_ended(end)
{
	return end.defect !== undefined ? _Runtime.die(end.defect)
		: end.error !== undefined ? _Runtime.fail(end.error)
		: _Runtime.succeed(__Maybe_Nothing);
}

// The next line, without its `\n` or `\r\n`; once the input ends, the text
// after the last newline is a line too.
function _NomiStream_nextLine(reader)
{
	var offset = 0;
	for (var i = 0; i < reader.chunks.length; i++)
	{
		var view = _NomiStream_view(reader.chunks[i]);
		var at = view.indexOf(10, Math.max(0, reader.scanned - offset));
		if (at >= 0)
		{
			return _NomiStream_line(reader, offset + at, 1);
		}
		offset += view.length;
	}
	reader.scanned = offset;
	if (reader.end === null)
	{
		return null;
	}
	return offset > 0 && reader.end.error === undefined && reader.end.defect === undefined
		? _NomiStream_line(reader, offset, 0)
		: _NomiStream_ended(reader.end);
}

var _NomiStream_utf8 = new TextDecoder('utf-8', { fatal: true, ignoreBOM: true });

// Takes the line's `length` bytes and its newline's from the buffer, and
// reads them as UTF-8.
function _NomiStream_line(reader, length, newline)
{
	var taken = _NomiStream_view(_NomiStream_splice(reader, length + newline));
	var end = newline && length > 0 && taken[length - 1] === 13 ? length - 1 : length;
	try
	{
		return _Runtime.succeed(__Maybe_Just(_NomiStream_utf8.decode(taken.subarray(0, end))));
	}
	catch (e)
	{
		return _Runtime.fail(__Stream_Invalid('a line is not UTF-8'));
	}
}

// The buffer's first `length` bytes, which it gives up.
function _NomiStream_splice(reader, length)
{
	reader.scanned = 0;
	var first = reader.chunks[0];
	if (first.byteLength === length)
	{
		return reader.chunks.shift();
	}
	if (first.byteLength > length)
	{
		reader.chunks[0] = new DataView(first.buffer, first.byteOffset + length, first.byteLength - length);
		return new DataView(first.buffer, first.byteOffset, length);
	}
	var joined = new Uint8Array(length);
	for (var at = 0; at < length; )
	{
		var view = _NomiStream_view(reader.chunks[0]);
		var size = Math.min(view.length, length - at);
		joined.set(view.subarray(0, size), at);
		at += size;
		size === view.length
			? reader.chunks.shift()
			: (reader.chunks[0] = _NomiStream_bytes(view.subarray(size)));
	}
	return _NomiStream_bytes(joined);
}

// Hands the reads what a source gave: a chunk, or `null` at the end.
function _NomiStream_give(reader, chunk)
{
	if (chunk === null)
	{
		reader.end = reader.end || {};
	}
	else if (chunk.byteLength > 0)
	{
		reader.chunks.push(chunk);
	}
	_NomiStream_deliver(reader);
}

function _NomiStream_fail(reader, end)
{
	reader.end = reader.end || end;
	_NomiStream_deliver(reader);
}

// A reader holding `bytes`, then at its end.
function _NomiStream_fromBytes(bytes)
{
	return _Runtime.sync(function() {
		return _NomiStream_held(bytes);
	});
}

function _NomiStream_held(bytes)
{
	var reader = _NomiStream_reader(false, function() {});
	_NomiStream_give(reader, bytes);
	_NomiStream_give(reader, null);
	return reader;
}

// A reader of the chunks `source` gives, `Nothing` ending it. A fiber,
// forked where the reader is made, so that it ends with the scope of the
// fiber that made it, runs `source` once a read waits that the buffer
// can't answer, one run at a time, and keeps what it gives for the reads.
// A read interrupted while `source` runs loses nothing: the chunk is the
// next read's. A source that fails or dies ends the reader so, and one
// whose fiber ended leaves it `Closed`.
function _NomiStream_fromTask(source)
{
	var reader = _NomiStream_reader(false, function(reader) {
		var wake = reader.wake;
		if (wake && _NomiStream_wanting(reader))
		{
			reader.wake = null;
			wake(_Runtime.succeed(__Utils_Tuple0));
		}
	});
	reader.wake = null;
	var wanted = _Runtime.wait(function(resume) {
		reader.wake = resume;
		reader.demand(reader);
		return function() { reader.wake = null; };
	});
	var pulled = _Runtime.onError(
		_Runtime.map(source, function(chunk) {
			_NomiStream_give(reader, __Maybe_isJust(chunk) ? chunk.a : null);
		}),
		function(error) {
			return _Runtime.sync(function() { _NomiStream_fail(reader, { error: error }); });
		}
	);
	var loop = _Runtime.andThen(wanted, function() {
		return _Runtime.andThen(pulled, function() {
			return reader.end === null ? loop : _Runtime.succeed(__Utils_Tuple0);
		});
	});
	var ended = _Runtime.addFinalizer(function(exit) {
		return _Runtime.sync(function() {
			_NomiStream_fail(reader, exit.$ === _Runtime.ExitKind.DEFECT ? { defect: exit.defect } : { error: __Stream_Closed });
		});
	});
	var pump = _Runtime.scoped(_Runtime.andThen(ended, function() { return loop; }));
	// The fiber's first step registers how it ends before its maker goes
	// on, which might end its scope before the fiber ran at all.
	return _Runtime.andThen(_Runtime.fork(pump), function() {
		return _Runtime.map(_Runtime.yieldNow, function() { return reader; });
	});
}


// A reader of a web `ReadableStream`, as Bun gives a child's output or a
// body: one read of it at a time, and only while a read waits, so what
// nobody asked for stays in the stream, which holds its source back. A
// chunk a read was waiting for when it was interrupted is the next read's.
function _NomiStream_fromWeb(stream)
{
	var source = stream.getReader();
	var reading = false;
	return _NomiStream_reader(true, function(reader) {
		if (reading || !_NomiStream_wanting(reader)) return;
		reading = true;
		source.read().then(function(read) {
			reading = false;
			_NomiStream_give(reader, read.done ? null : _NomiStream_bytes(read.value));
		}, function(error) {
			reading = false;
			_NomiStream_fail(reader, { error: _NomiStream_error(error) });
		});
	});
}


// LINES

function _NomiStream_lines(reader)
{
	return _Runtime.succeed({ reader: reader });
}

function _NomiStream_lineOf(lines)
{
	return _NomiStream_take(lines.reader, true);
}

function _NomiStream_readLine(reader)
{
	return _NomiStream_take(reader, true);
}


// WRITERS

// A writer: whether it is closed, whether a write or the close has its
// turn, the fibers waiting for theirs, oldest first, and its sink:
// `write(bytes)` and `close`, tasks, whether it is a terminal, and its
// `size()` if it is one. Writes take turns, so they reach the sink in the
// order they came, and each waits for the sink to take its bytes.
function _NomiStream_writer(sink)
{
	return { closed: false, busy: false, waiting: [], sink: sink };
}

// Runs `task()` once the writer is the fiber's, then passes the writer on,
// however the task ends.
function _NomiStream_turn(writer, task)
{
	var turn = _Runtime.wait(function(resume) {
		if (!writer.busy)
		{
			writer.busy = true;
			resume(_Runtime.succeed(__Utils_Tuple0));
			return;
		}
		writer.waiting.push(resume);
		return function() { _NomiStream_remove(writer.waiting, resume); };
	});
	return _Runtime.andThen(turn, function() {
		return _Runtime.ensuring(task(), _Runtime.sync(function() {
			var next = writer.waiting.shift();
			next ? next(_Runtime.succeed(__Utils_Tuple0)) : (writer.busy = false);
			return __Utils_Tuple0;
		}));
	});
}

var _NomiStream_write = F2(function(bytes, writer)
{
	return _NomiStream_turn(writer, function() {
		return writer.closed ? _Runtime.fail(__Stream_Closed) : writer.sink.write(bytes);
	});
});

function _NomiStream_close(writer)
{
	return _NomiStream_turn(writer, function() {
		if (writer.closed)
		{
			return _Runtime.succeed(__Utils_Tuple0);
		}
		writer.closed = true;
		return writer.sink.close;
	});
}

// A writer to Bun's `FileSink`, as a child's input: a write waits until
// the sink has flushed it, which a reader that doesn't read holds back.
function _NomiStream_toSink(sink)
{
	var done = function(f) {
		return _Runtime.onError(_Runtime.map(_Runtime.fromPromise(f), function() { return __Utils_Tuple0; }), function(error) {
			return _Runtime.fail(_NomiStream_error(error));
		});
	};
	return _NomiStream_writer({
		write: function(bytes) {
			return done(function() {
				sink.write(_NomiStream_view(bytes));
				return sink.flush();
			});
		},
		close: done(function() { return sink.end(); }),
		terminal: false,
		size: function() { return null; }
	});
}

// A writer running the tasks of `sink`, an Elm record.
function _NomiStream_fromTasks(sink)
{
	return _Runtime.sync(function() {
		return _NomiStream_writer({
			write: sink.__$write,
			close: sink.__$close,
			terminal: false,
			size: function() { return null; }
		});
	});
}

// A writer keeping what is written, as a test's output does: a terminal
// of `size` when one is given.
function _NomiStream_capture(size)
{
	size = size || null;
	var captured = [];
	var writer = _NomiStream_writer({
		write: function(bytes) {
			return _Runtime.sync(function() {
				captured.push(bytes);
				return __Utils_Tuple0;
			});
		},
		close: _Runtime.succeed(__Utils_Tuple0),
		terminal: size !== null,
		size: function() { return size; }
	});
	writer.captured = captured;
	return writer;
}

// What a capturing writer kept, as UTF-8 text, each byte that isn't
// replaced with U+FFFD.
function _NomiStream_captured(writer)
{
	var size = 0;
	for (var i = 0; i < writer.captured.length; i++)
	{
		size += writer.captured[i].byteLength;
	}
	var all = new Uint8Array(size);
	for (var i = 0, at = 0; i < writer.captured.length; at += writer.captured[i].byteLength, i++)
	{
		all.set(_NomiStream_view(writer.captured[i]), at);
	}
	return new TextDecoder().decode(all);
}


// TERMINALS

function _NomiStream_isTerminal(writer)
{
	return _Runtime.sync(function() {
		return writer.sink.terminal;
	});
}

function _NomiStream_isInteractive(reader)
{
	return _Runtime.sync(function() {
		return reader.interactive;
	});
}

function _NomiStream_size(writer)
{
	return _Runtime.sync(function() {
		var size = writer.sink.size();
		return size === null ? __Maybe_Nothing : __Maybe_Just({ __$columns: size.columns, __$rows: size.rows });
	});
}
