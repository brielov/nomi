/*

import Elm.Kernel.List exposing (fromArray, toArray)
import Elm.Kernel.NomiFs exposing (cwd)
import Elm.Kernel.NomiNet exposing (real)
import Elm.Kernel.NomiProcess exposing (real)
import Elm.Kernel.NomiStdio exposing (stdin, stdout, stderr)
import Elm.Kernel.Platform exposing (export)
import Elm.Kernel.Scheduler exposing (runtime)
import Elm.Kernel.Utils exposing (Tuple0, Tuple2)
import Maybe exposing (Just, Nothing)
import Nomi.Fiber as Fiber exposing (Success, Failure, Interrupted, Defect)

*/


// PROGRAMS

// A program's `main`, applied as every `main` is to its flags' decoder and
// the debugger's metadata. Its `init` runs the task `run` makes of the
// process's arguments, variables, working directory and standard streams
// on the program's runtime, under the runtime's `main`, which owns the
// process from then on: its exit code, and SIGINT and SIGTERM. The `program` mark tells an entry's shim that
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
			var task = A8(run, args, vars, __NomiFs_cwd(), __NomiNet_real, __NomiProcess_real, __NomiStdio_stdin, __NomiStdio_stdout, __NomiStdio_stderr);
			_Runtime.main(task, { runtime: __Scheduler_runtime });
		}
		init.program = true;
		return init;
	});
}

// Every `main` is exported through Platform's kernel, which upstream's
// programs reach through `_Platform_initialize` and a nomi program here.
void __Platform_export;


// The task asks the program's lifecycle to stop it and exit with `code`,
// then waits to be interrupted; the runtime has no lifecycle outside a
// program, and the task dies there. A code past what a process can exit
// with is 255, not the failure's code wrapped around to another, or to 0.
function _Nomi_exitWith(code)
{
	return _Runtime.wait(function(resume) {
		if (!_Runtime.requestExit(__Scheduler_runtime, code >= 0 && code <= 255 ? code : 255))
		{
			resume(_Runtime.die(new Error('Nomi.Process.exit is for a Nomi.program')));
		}
	});
}


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


// REFS

function _Nomi_makeRef(value)
{
	return _Runtime.sync(function() { return { value: value }; });
}

function _Nomi_getRef(ref)
{
	return _Runtime.sync(function() { return ref.value; });
}

var _Nomi_setRef = F2(function(value, ref)
{
	return _Runtime.sync(function() { ref.value = value; return __Utils_Tuple0; });
});

var _Nomi_updateRef = F2(function(f, ref)
{
	return _Runtime.sync(function() { ref.value = f(ref.value); return __Utils_Tuple0; });
});

// One step, so no other fiber runs between the read and the write.
var _Nomi_modifyRef = F2(function(f, ref)
{
	return _Runtime.sync(function() {
		var pair = f(ref.value);
		ref.value = pair.b;
		return pair.a;
	});
});


// QUEUES

// Items, at most `capacity` of them, and the fibers waiting to take or to
// offer, oldest first. Takers wait only while there are no items, and
// offerers only while the queue is full; each waits through the runtime's
// `wait`, so whatever is handed to it is never dropped by an interruption,
// and an interrupted waiter's cancel takes it out of line.
function _Nomi_queue(capacity)
{
	return _Runtime.sync(function() {
		return { capacity: capacity, items: [], takers: [], offerers: [] };
	});
}

function _Nomi_bounded(capacity)
{
	return _Nomi_queue(Math.max(0, capacity));
}

var _Nomi_unbounded = _Nomi_queue(Infinity);

var _Nomi_offer = F2(function(item, queue)
{
	return _Nomi_defer(function() {
		var taker = queue.takers.shift();
		if (taker)
		{
			taker(_Runtime.succeed(item));
			return _Runtime.succeed(__Utils_Tuple0);
		}
		if (queue.items.length < queue.capacity)
		{
			queue.items.push(item);
			return _Runtime.succeed(__Utils_Tuple0);
		}
		return _Nomi_waitIn(queue.offerers, function(resume) {
			return { item: item, resume: resume };
		});
	});
});

function _Nomi_take(queue)
{
	return _Nomi_defer(function() {
		var item = _Nomi_next(queue);
		return item ? _Runtime.succeed(item.value) : _Nomi_waitIn(queue.takers, function(resume) {
			return resume;
		});
	});
}

function _Nomi_poll(queue)
{
	return _Runtime.sync(function() {
		var item = _Nomi_next(queue);
		return item ? __Maybe_Just(item.value) : __Maybe_Nothing;
	});
}

// The oldest item, which lets the oldest offerer in, or an offerer's item
// directly when the queue holds none; `undefined` when there is neither.
function _Nomi_next(queue)
{
	var offerer;
	if (queue.items.length > 0)
	{
		var value = queue.items.shift();
		offerer = queue.offerers.shift();
		if (offerer)
		{
			queue.items.push(offerer.item);
			offerer.resume(_Runtime.succeed(__Utils_Tuple0));
		}
		return { value: value };
	}
	offerer = queue.offerers.shift();
	if (offerer)
	{
		offerer.resume(_Runtime.succeed(__Utils_Tuple0));
		return { value: offerer.item };
	}
	return undefined;
}

// Waits at the back of `line` as `waiter(resume)`, leaving the line if
// interrupted.
function _Nomi_waitIn(line, waiter)
{
	return _Runtime.wait(function(resume) {
		var entry = waiter(resume);
		line.push(entry);
		return function() {
			line.splice(line.indexOf(entry), 1);
		};
	});
}

// The task `f` gives, called when the task runs.
function _Nomi_defer(f)
{
	return _Runtime.andThen(_Runtime.succeed(__Utils_Tuple0), f);
}
