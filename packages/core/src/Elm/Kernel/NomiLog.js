/*

import Elm.Kernel.List exposing (toArray)
import Elm.Kernel.NomiStdio exposing (logs, stderr, print)
import Elm.Kernel.Utils exposing (Tuple0)

*/


// LINES

// Writes a line of `level` (its name), the message and the fields, each a
// name and a value, to the stderr of whatever runs the task: the
// process's, or under `nomi test` the test's. On a terminal the line is
// text; anywhere else it is JSON. Its time is the program's clock's. A
// line is one write, so lines never mix, and a log line can't fail.
var _NomiLog_write = F3(function(level, message, fields)
{
	return _Runtime.withRuntime(function(runtime) {
		return _Runtime.andThen(_Runtime.now, function(now) {
			var writer = __NomiStdio_logs.get(runtime) || __NomiStdio_stderr;
			var pairs = __List_toArray(fields).map(function(pair) { return [pair.a, pair.b.a]; });
			var line = writer.sink.terminal ? _NomiLog_text(now, level, message, pairs) : _NomiLog_json(now, level, message, pairs);
			return _Runtime.onError(A2(__NomiStdio_print, line + '\n', writer), function() {
				return _Runtime.succeed(__Utils_Tuple0);
			});
		});
	});
});

// The line's own keys, which a field of the same name doesn't replace:
// the field is written with a `_` before its name.
var _NomiLog_own = { time: true, level: true, message: true };

function _NomiLog_json(now, level, message, pairs)
{
	var line = { time: new Date(now).toISOString(), level: level, message: message };
	pairs.forEach(function(pair) {
		line[_NomiLog_own[pair[0]] ? '_' + pair[0] : pair[0]] = pair[1];
	});
	return JSON.stringify(line);
}

var _NomiLog_colours = { debug: '\u001b[2m', info: '\u001b[32m', warn: '\u001b[33m', error: '\u001b[31m' };

// `12:03:04.123 INFO  handled status=200 path="/todos"`: the time of day,
// the level coloured, the message, and each field, its text quoted when
// it holds a space, a quote or nothing.
function _NomiLog_text(now, level, message, pairs)
{
	var time = new Date(now).toISOString().slice(11, 23);
	var name = level.toUpperCase().padEnd(5);
	var shown = pairs.map(function(pair) {
		var value = pair[1];
		var text = typeof value === 'string' && (value === '' || /[\s"=\\]/.test(value)) ? JSON.stringify(value) : String(value);
		return pair[0] + '=' + text;
	});
	return [time, _NomiLog_colours[level] + name + '\u001b[0m', message].concat(shown).join(' ');
}
