/*

import Elm.Kernel.Utils exposing (Tuple0)

*/


// elm/core's scheduler on the fiber runtime (runtime/src/fiber.js), which
// the program's prologue holds as `_Runtime`. A process is a fiber and a
// task the runtime's own, and every name other kernels call keeps
// upstream's meaning. One runtime runs the program, draining at once when
// work arrives outside a drain, as upstream's `_Scheduler_enqueue` does,
// so a port answers before its `send` returns. It runs on the real clock;
// nomi/test runs each effectful test on a runtime of its own, on a
// virtual clock.

var _Scheduler_runtime = new _Runtime.Runtime({ synchronous: true });


// TASKS

var _Scheduler_succeed = _Runtime.succeed;

var _Scheduler_fail = _Runtime.fail;

var _Scheduler_binding = _Runtime.async;

var _Scheduler_andThen = F2(function(callback, task)
{
	return _Runtime.andThen(task, callback);
});

var _Scheduler_onError = F2(function(callback, task)
{
	return _Runtime.onError(task, callback);
});

var _Scheduler_receive = _Runtime.receive;


// PROCESSES

// A spawned process outlives its spawner, as a daemon does.
function _Scheduler_rawSpawn(task)
{
	return _Scheduler_runtime.fork(task);
}

var _Scheduler_spawn = _Runtime.forkDaemon;

function _Scheduler_rawSend(proc, msg)
{
	_Scheduler_runtime.send(proc, msg);
}

var _Scheduler_send = F2(function(proc, msg)
{
	return _Runtime.sync(function() {
		_Scheduler_rawSend(proc, msg);
		return __Utils_Tuple0;
	});
});

// Upstream's kill stops the process and succeeds without waiting for it.
function _Scheduler_kill(proc)
{
	return _Runtime.map(_Runtime.interruptFork(proc), function() {
		return __Utils_Tuple0;
	});
}
