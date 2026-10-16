/*

import Elm.Kernel.List exposing (Nil)
import Elm.Kernel.NomiFs exposing (memory, copy)
import Elm.Kernel.NomiStdio exposing (script, capture, captured, logs)
import Elm.Kernel.Utils exposing (Tuple0)
import Maybe exposing (Just, Nothing)
import Nomi exposing (Net)
import Result exposing (Ok, Err)

*/


// EFFECTFUL TESTS

// The task of each effectful test's placeholder expectation, which a test
// makes anew each time it runs: the side table nomi's runner reads
// through `Nomi.Test.Runner`.
var _NomiTest_effects = new WeakMap();

var _NomiTest_mark = F2(function(task, placeholder)
{
	_NomiTest_effects.set(placeholder, task);
	return placeholder;
});

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

// `make` applied to a file system in memory, an input that has ended, two
// captured outputs and the network.
function _NomiTest_env(make)
{
	return A5(make, __NomiFs_memory(__List_Nil), __NomiStdio_script(''), __NomiStdio_capture(), __NomiStdio_capture(), __Nomi_Net);
}

// `make` applied to a copy of `fs`, an input that has ended and two new
// captured outputs, made when the task runs, so each run of a test has
// its own. The test's runtime is its own, and `Nomi.log` on it writes to
// the captured stderr.
var _NomiTest_fresh = F2(function(make, fs)
{
	return _Runtime.withRuntime(function(runtime) {
		var stderr = __NomiStdio_capture();
		__NomiStdio_logs.set(runtime, stderr);
		return _Runtime.succeed(A4(make, A2(__NomiFs_copy, __List_Nil, fs), __NomiStdio_script(''), __NomiStdio_capture(), stderr));
	});
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
