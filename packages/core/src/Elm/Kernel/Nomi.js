/*

import Elm.Kernel.List exposing (fromArray, toArray)
import Elm.Kernel.Platform exposing (export)
import Elm.Kernel.Scheduler exposing (runtime)
import Elm.Kernel.Utils exposing (Tuple2)
import Maybe exposing (Just, Nothing)
import Nomi.Fiber as Fiber exposing (Success, Failure, Interrupted, Defect)

*/


// PROGRAMS

// A program's `main`, applied as every `main` is to its flags' decoder and
// the debugger's metadata. Its `init` runs the task `run` makes of the
// process's arguments and variables on the program's runtime, under the
// runtime's `main`, which owns the process from then on: its exit code,
// and SIGINT and SIGTERM. The `program` mark tells an entry's shim that
// `init` starts that lifecycle, so the shim starts no worker's.
function _Nomi_program(run)
{
	return F2(function(flagDecoder, debugMetadata)
	{
		function init()
		{
			var args = __List_fromArray(process.argv.slice(2));
			var vars = __List_fromArray(Object.entries(process.env).map(function(entry) {
				return __Utils_Tuple2(entry[0], entry[1]);
			}));
			_Runtime.main(A2(run, args, vars), { runtime: __Scheduler_runtime });
		}
		init.program = true;
		return init;
	});
}

// Every `main` is exported through Platform's kernel, which upstream's
// programs reach through `_Platform_initialize` and a nomi program here.
void __Platform_export;


// TASKS

var _Nomi_both = F2(function(a, b)
{
	return _Runtime.map(_Runtime.all([a, b]), function(values) {
		return __Utils_Tuple2(values[0], values[1]);
	});
});

function _Nomi_all(tasks)
{
	return _Runtime.map(_Runtime.all(__List_toArray(tasks)), __List_fromArray);
}

var _Nomi_race = F2(function(a, b)
{
	return _Runtime.race([a, b]);
});

// The task races a sleep on the program's clock.
var _Nomi_timeout = F2(function(ms, task)
{
	return _Runtime.race([
		_Runtime.map(task, __Maybe_Just),
		_Runtime.map(_Runtime.sleep(ms), function() { return __Maybe_Nothing; })
	]);
});

// The resource's scope is the use's.
var _Nomi_acquireRelease = F3(function(acquire, release, use)
{
	var released = _Runtime.acquireRelease(acquire, function(resource) {
		return release(resource);
	});
	return _Runtime.scoped(_Runtime.andThen(released, use));
});

var _Nomi_ensure = F2(function(finalizer, task)
{
	return _Runtime.ensuring(task, finalizer);
});

var _Nomi_uninterruptible = _Runtime.uninterruptible;


// FIBERS

var _Nomi_fork = _Runtime.fork;

var _Nomi_forkDaemon = _Runtime.forkDaemon;

var _Nomi_join = _Runtime.join;

function _Nomi_await(fiber)
{
	return _Runtime.map(_Runtime.awaitFiber(fiber), _Nomi_exit);
}

function _Nomi_interrupt(fiber)
{
	return _Runtime.map(_Runtime.interrupt(fiber), _Nomi_exit);
}

// A runtime exit as a `Nomi.Fiber.Exit`; a defect is described by
// `String`, which gives an exception's name and message.
function _Nomi_exit(exit)
{
	switch (exit.$)
	{
		case _Runtime.ExitKind.SUCCESS:
			return __Fiber_Success(exit.value);
		case _Runtime.ExitKind.FAIL:
			return __Fiber_Failure(exit.error);
		case _Runtime.ExitKind.INTERRUPTED:
			return __Fiber_Interrupted;
		default:
			return __Fiber_Defect(String(exit.defect));
	}
}
