/*

import Elm.Kernel.List exposing (Nil)
import Elm.Kernel.NomiFs exposing (memory, copy)
import Elm.Kernel.NomiStdio exposing (script, capture, captured, logs)
import Elm.Kernel.NomiStream exposing (bytes)
import Elm.Kernel.Utils exposing (Tuple0)
import Maybe exposing (Just, Nothing)
import Elm.Kernel.NomiNet exposing (real)
import Result exposing (Ok, Err)

*/


// EFFECTFUL TESTS

// The task of each effectful test's placeholder expectation, which a test
// makes anew each time it runs: the side table nomi's runner reads
// through `Nomi.Test.Runner`.
var _NomiTest_effects = new WeakMap();

// Each effectful test's seed, by its task: its description, hashed, so a
// test's random values are its own and the same on every run.
var _NomiTest_seeds = new WeakMap();

var _NomiTest_mark = F3(function(description, task, placeholder)
{
	_NomiTest_effects.set(placeholder, task);
	_NomiTest_seeds.set(task, _NomiTest_hash(description));
	return placeholder;
});

// FNV-1a over the text's UTF-16 units.
function _NomiTest_hash(text)
{
	var hash = 0x811c9dc5;
	for (var i = 0; i < text.length; i++)
	{
		hash = Math.imul(hash ^ text.charCodeAt(i), 0x01000193) >>> 0;
	}
	return hash;
}

// A generator of bytes from `seed`, splitmix32's, which fills an array as
// `crypto.getRandomValues` does.
function _NomiTest_generator(seed)
{
	var state = seed >>> 0;
	return function(array) {
		for (var i = 0; i < array.length; i++)
		{
			state = (state + 0x9e3779b9) >>> 0;
			var z = state;
			z = Math.imul(z ^ (z >>> 16), 0x21f0aaad);
			z = Math.imul(z ^ (z >>> 15), 0x735a2d97);
			array[i] = (z ^ (z >>> 15)) & 0xff;
		}
		return array;
	};
}

function _NomiTest_effect(expectation)
{
	var task = _NomiTest_effects.get(expectation);
	return task ? __Maybe_Just(task) : __Maybe_Nothing;
}

// The task runs on a runtime of its own, on a virtual clock from 0, so
// its time moves only when every one of its fibers waits on a timer,
// whatever another test or the runner does. Once it ends, whatever it
// left running, as a daemon, is interrupted before its outcome is known.
// Its exit is the test's, a defect the test's failure rather than the
// runner's.
function _NomiTest_attempt(task)
{
	return _Runtime.async(function(resume) {
		var runtime = new _Runtime.Runtime({ clock: new _Runtime.TestClock() });
		runtime.random = _NomiTest_generator(_NomiTest_seeds.get(task) || 0);
		runtime.fork(task, function(exit) {
			runtime.shutdown().then(function() {
				resume(_Runtime.succeed(_NomiTest_result(exit)));
			});
		});
		return function() { runtime.shutdown(); };
	});
}

function _NomiTest_result(exit)
{
	switch (exit.$)
	{
		case _Runtime.ExitKind.SUCCESS:
			return __Result_Ok(exit.value);
		case _Runtime.ExitKind.FAIL:
			return __Result_Err(exit.error);
		case _Runtime.ExitKind.INTERRUPTED:
			return __Result_Err('This test failed because its task was interrupted.');
		default:
			return __Result_Err('This test failed because its task died: "' + String(exit.defect) + '"');
	}
}

// Waits `ms` of the test's clock, and then until whatever woke by then
// waits again: a zero delay on the virtual clock is due once every fiber
// waits.
function _NomiTest_advance(ms)
{
	return _Runtime.andThen(_Runtime.sleep(ms), function() {
		return _Runtime.map(_Runtime.sleep(0), function() {
			return __Utils_Tuple0;
		});
	});
}


// ENVIRONMENTS

var _NomiTest_nothing = new DataView(new ArrayBuffer(0));

// `make` applied to a file system in memory, processes that run nothing,
// an input that has ended, two captured outputs and the network.
function _NomiTest_env(make)
{
	return A6(make, __NomiFs_memory(__List_Nil), _NomiTest_commands(function() { return __Maybe_Nothing; }), __NomiStdio_script(_NomiTest_nothing), __NomiStdio_capture(null), __NomiStdio_capture(null), __NomiNet_real);
}

// Fake processes answering with `answer`, which each run of a test makes
// anew, with nothing run yet. The system's are the system's.
function _NomiTest_commands(answer)
{
	return { fake: answer, ran: [] };
}

function _NomiTest_freshProcesses(processes)
{
	return processes.fake === null ? processes : _NomiTest_commands(processes.fake);
}

// `make` applied to a copy of `fs`, the bytes `stdin` was scripted with
// from its start, and two new captured outputs, terminals as the
// template's are, made when the task runs, so each run of a test has its
// own. An input not scripted, as the process's, is one that has ended.
// The test's runtime is its own, and `Nomi.log` on it writes to the
// captured stderr.
var _NomiTest_fresh = F6(function(make, fs, processes, stdin, stdout, stderr)
{
	return _Runtime.withRuntime(function(runtime) {
		var err = __NomiStdio_capture(stderr.sink.size());
		__NomiStdio_logs.set(runtime, err);
		var input = __NomiStdio_script(stdin.script === undefined ? _NomiTest_nothing : stdin.script);
		input.interactive = stdin.interactive;
		return _Runtime.succeed(A5(make, A2(__NomiFs_copy, __List_Nil, fs), _NomiTest_freshProcesses(processes), input, __NomiStdio_capture(stdout.sink.size()), err));
	});
});

var _NomiTest_encoder = new TextEncoder();

function _NomiTest_stdin(text)
{
	return __NomiStdio_script(__NomiStream_bytes(_NomiTest_encoder.encode(text)));
}

function _NomiTest_stdinBytes(bytes)
{
	return __NomiStdio_script(bytes);
}

// The input, an interactive copy, and the outputs, terminals of `size`.
var _NomiTest_terminal = F3(function(make, size, stdin)
{
	var input = __NomiStdio_script(stdin.script === undefined ? _NomiTest_nothing : stdin.script);
	input.interactive = true;
	var columnsRows = { columns: size.__$columns, rows: size.__$rows };
	return A3(make, input, __NomiStdio_capture(columnsRows), __NomiStdio_capture(columnsRows));
});

var _NomiTest_withFiles = F2(function(files, fs)
{
	return A2(__NomiFs_copy, files, fs);
});

function _NomiTest_captured(writer)
{
	return _Runtime.sync(function() {
		return __NomiStdio_captured(writer);
	});
}
