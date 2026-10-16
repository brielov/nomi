/*

import Elm.Kernel.List exposing (Nil)
import Elm.Kernel.NomiFs exposing (memory)
import Elm.Kernel.NomiStdio exposing (script, capture)
import Maybe exposing (Just, Nothing)
import Nomi exposing (Net)
import Result exposing (Ok, Err)

*/


// EFFECTFUL TESTS

// The task of each effectful test's placeholder expectation, which a test
// makes anew each time it runs: the side table nomi's runner reads
// through `Nomi.Test.Runner`.
var _NomiTest_effects = new WeakMap();

var _NomiTest_mark = F2(function(run, placeholder)
{
	_NomiTest_effects.set(placeholder, run);
	return placeholder;
});

function _NomiTest_effect(expectation)
{
	var run = _NomiTest_effects.get(expectation);
	return run ? __Maybe_Just(run) : __Maybe_Nothing;
}

// The task runs on a fiber of its own, whose exit the test waits for, so a
// defect is the test's failure rather than the runner's.
function _NomiTest_attempt(task)
{
	return _Runtime.andThen(_Runtime.fork(task), function(fiber) {
		return _Runtime.map(_Runtime.awaitFiber(fiber), _NomiTest_result);
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


// ENVIRONMENTS

// `make` applied to a new file system in memory, an input that has ended,
// two captured outputs and the network.
function _NomiTest_env(make)
{
	return _Runtime.sync(function() {
		return A5(make, __NomiFs_memory(__List_Nil), __NomiStdio_script(''), __NomiStdio_capture(), __NomiStdio_capture(), __Nomi_Net);
	});
}
