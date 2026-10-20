/*

import Dict exposing (toList)
import Elm.Kernel.List exposing (fromArray, toArray)
import Elm.Kernel.NomiBytes exposing (join)
import Elm.Kernel.NomiStream exposing (held, fromWeb, toSink, writer, capture, view)
import Elm.Kernel.Utils exposing (Tuple0)
import Maybe exposing (Just, Nothing, isJust)
import Nomi.Process as Process exposing (NotFound, PermissionDenied, Other, Linux, MacOs, Windows, OtherPlatform, Hangup, Interrupt, Quit, Kill, User1, User2, Terminate, WindowChanged)

*/


// THE PROCESS

var _NomiProcess_cwd = _Runtime.sync(function() {
	return process.cwd();
});

var _NomiProcess_pid = _Runtime.sync(function() {
	return process.pid;
});

var _NomiProcess_platform = _Runtime.sync(function() {
	switch (process.platform)
	{
		case 'linux':
			return __Process_Linux;
		case 'darwin':
			return __Process_MacOs;
		case 'win32':
			return __Process_Windows;
		default:
			return __Process_OtherPlatform(process.platform);
	}
});

var _NomiProcess_windows = process.platform === 'win32';


// PROCESSES

// The capability to run programs: the system's, or a fake's function and
// the commands it ran, each with the chunks given on its standard input.
var _NomiProcess_real = { fake: null, ran: [] };

function _NomiProcess_fake(answer)
{
	return _Runtime.sync(function() {
		return { fake: answer, ran: [] };
	});
}

function _NomiProcess_ran(processes)
{
	return _Runtime.sync(function() {
		return __List_fromArray(processes.ran.map(function(entry) {
			return { __$command: entry.command, __$stdin: __NomiBytes_join(entry.stdin) };
		}));
	});
}


// RUNNING

// Runs `command`, whose fields are `c`, to its end, in a scope of its own,
// so its child is ended once the task is, however it is, and no later.
var _NomiProcess_output = F3(function(command, c, processes)
{
	var run = _Runtime.andThen(_NomiProcess_owned(command, c, processes, false), function(child) {
		return _Runtime.andThen(child.wait, function(status) {
			return _Runtime.map(_Runtime.fromPromise(function() { return child.collected; }), function(both) {
				return { __$status: status, __$stdout: both[0], __$stderr: both[1] };
			});
		});
	});
	return _Runtime.scoped(run);
});

var _NomiProcess_spawn = F3(function(command, c, processes)
{
	return _NomiProcess_owned(command, c, processes, true);
});

function _NomiProcess_stdin(child)
{
	return child.stdin;
}

function _NomiProcess_stdout(child)
{
	return child.stdout;
}

function _NomiProcess_stderr(child)
{
	return child.stderr;
}

function _NomiProcess_wait(child)
{
	return child.wait;
}

var _NomiProcess_kill = F2(function(signal, child)
{
	return _Runtime.sync(function() {
		child.kill(_NomiProcess_name(signal));
		return __Utils_Tuple0;
	});
});

// A child of `command`, the fiber's that started it: once the fiber
// ends, it is ended as `end` does. A child whose output `output` collects
// reads it all as it comes, rather than as reads ask for it.
function _NomiProcess_owned(command, c, processes, spawned)
{
	var started = processes.fake === null
		? _Runtime.onError(
			_Runtime.fromPromise(function() { return _NomiProcess_start(c, spawned); }),
			function(error) { return _Runtime.fail(_NomiProcess_error(c, error)); }
		)
		: _NomiProcess_fakeChild(processes, command, c, spawned);
	return _Runtime.acquireRelease(started, function(child) {
		return _Runtime.onError(
			_Runtime.map(_Runtime.fromPromise(function() { return child.end(); }), function() { return __Utils_Tuple0; }),
			function() { return _Runtime.succeed(__Utils_Tuple0); }
		);
	});
}

var _NomiProcess_fs = process.getBuiltinModule('node:fs');

// Starts the command on the system, once its directory is known to be
// one, as Bun says only that something wasn't found.
function _NomiProcess_start(c, spawned)
{
	var dir = __Maybe_isJust(c.__$cwd) ? c.__$cwd.a : undefined;
	var checked = dir === undefined ? Promise.resolve() : _NomiProcess_fs.promises.stat(dir).then(function(stat) {
		if (!stat.isDirectory()) throw Object.assign(new Error('ENOTDIR'), { code: 'ENOTDIR', named: dir });
	}, function(error) {
		throw Object.assign(error, { named: dir });
	});
	return checked.then(function() {
		var given = __Maybe_isJust(c.__$stdin);
		var proc = Bun.spawn({
			cmd: [c.__$program].concat(__List_toArray(c.__$arguments)),
			cwd: dir,
			env: __Maybe_isJust(c.__$env) ? _NomiProcess_vars(c.__$env.a) : process.env,
			stdin: given ? __NomiStream_view(c.__$stdin.a) : spawned ? 'pipe' : 'ignore',
			stdout: 'pipe',
			stderr: 'pipe'
		});
		var exited = proc.exited;
		var child = {
			stdin: given || !spawned ? _NomiProcess_closed() : __NomiStream_toSink(proc.stdin),
			stdout: spawned ? __NomiStream_fromWeb(proc.stdout) : null,
			stderr: spawned ? __NomiStream_fromWeb(proc.stderr) : null,
			collected: spawned ? null : Promise.all([_NomiProcess_all(proc.stdout), _NomiProcess_all(proc.stderr)]),
			wait: _Runtime.fromPromise(function() { return exited; }),
			kill: function(name) {
				if (proc.exitCode === null && proc.signalCode === null) _NomiProcess_windows ? proc.kill() : proc.kill(name);
			},
			// SIGTERM, then SIGKILL once the grace passed, and the exit.
			end: function() {
				if (proc.exitCode !== null || proc.signalCode !== null) return exited;
				if (_NomiProcess_windows)
				{
					proc.kill();
					return exited;
				}
				proc.kill('SIGTERM');
				var timer;
				var grace = new Promise(function(resolve) { timer = setTimeout(resolve, c.__$grace); });
				return Promise.race([exited, grace]).then(function() {
					clearTimeout(timer);
					if (proc.exitCode === null && proc.signalCode === null) proc.kill('SIGKILL');
					return exited;
				});
			}
		};
		return child;
	});
}

function _NomiProcess_all(stream)
{
	return new Response(stream).arrayBuffer().then(function(buffer) {
		return new DataView(buffer);
	});
}

function _NomiProcess_vars(dict)
{
	var vars = {};
	__List_toArray(__Dict_toList(dict)).forEach(function(pair) { vars[pair.a] = pair.b; });
	return vars;
}

// A writer that is closed already, as a child's input given whole.
function _NomiProcess_closed()
{
	var closed = __NomiStream_capture(null);
	closed.closed = true;
	return closed;
}

function _NomiProcess_error(c, error)
{
	var name = error.named !== undefined ? error.named : c.__$program;
	switch (error.code)
	{
		case 'ENOENT':
			return __Process_NotFound(name);
		case 'EACCES':
		case 'EPERM':
			return __Process_PermissionDenied(name);
		case 'ENOTDIR':
			return __Process_Other(name + ': not a directory');
		default:
			return __Process_Other(name + ': ' + (error.message || error));
	}
}


// FAKE CHILDREN

// A child of a fake: its output what the fake's function gave, its input
// kept for `ran`, and its exit once its input is closed, or once it is
// killed or its fiber ends.
function _NomiProcess_fakeChild(processes, command, c, spawned)
{
	var answered = _Runtime.sync(function() { return processes.fake(command); });
	return _Runtime.andThen(answered, function(answer) {
		// A command the fake doesn't know is a program there is none of.
		return __Maybe_isJust(answer)
			? _Runtime.succeed(_NomiProcess_fakeOf(processes, command, c, spawned, answer.a))
			: _Runtime.fail(__Process_NotFound(c.__$program));
	});
}

function _NomiProcess_fakeOf(processes, command, c, spawned, given)
{
	var entry = { command: command, stdin: [] };
	processes.ran.push(entry);
	var child = { status: null, waiting: [] };
	var exit = function(status) {
		if (child.status !== null) return;
		child.status = status;
		child.waiting.splice(0).forEach(function(resume) { resume(_Runtime.succeed(status)); });
	};
	if (__Maybe_isJust(c.__$stdin) || !spawned)
	{
		__Maybe_isJust(c.__$stdin) && entry.stdin.push(c.__$stdin.a);
		child.stdin = _NomiProcess_closed();
		exit(given.__$status);
	}
	else
	{
		child.stdin = __NomiStream_writer({
			write: function(bytes) {
				return _Runtime.sync(function() {
					entry.stdin.push(bytes);
					return __Utils_Tuple0;
				});
			},
			close: _Runtime.sync(function() {
				exit(given.__$status);
				return __Utils_Tuple0;
			}),
			terminal: false,
			size: function() { return null; }
		});
	}
	child.stdout = __NomiStream_held(given.__$stdout);
	child.stderr = __NomiStream_held(given.__$stderr);
	child.collected = Promise.resolve([given.__$stdout, given.__$stderr]);
	child.wait = _Runtime.wait(function(resume) {
		if (child.status !== null)
		{
			resume(_Runtime.succeed(child.status));
			return;
		}
		child.waiting.push(resume);
		return function() {
			var at = child.waiting.indexOf(resume);
			at < 0 || child.waiting.splice(at, 1);
		};
	});
	child.kill = function(name) {
		name === 'SIGWINCH' || exit(128 + _NomiProcess_numbers[name]);
	};
	child.end = function() {
		exit(128 + _NomiProcess_numbers.SIGTERM);
		return Promise.resolve();
	};
	return child;
}


// SIGNALS

var _NomiProcess_numbers = { SIGHUP: 1, SIGINT: 2, SIGQUIT: 3, SIGKILL: 9, SIGUSR1: 10, SIGUSR2: 12, SIGTERM: 15, SIGWINCH: 28 };

function _NomiProcess_name(signal)
{
	return signal === __Process_Hangup ? 'SIGHUP'
		: signal === __Process_Interrupt ? 'SIGINT'
		: signal === __Process_Quit ? 'SIGQUIT'
		: signal === __Process_Kill ? 'SIGKILL'
		: signal === __Process_User1 ? 'SIGUSR1'
		: signal === __Process_User2 ? 'SIGUSR2'
		: signal === __Process_Terminate ? 'SIGTERM'
		: 'SIGWINCH';
}

// Signals the process listens for: those that came and no fiber took
// yet, each kind once, oldest first; the fibers waiting for one; and
// whether the fiber that listens ended. It listens until then.
function _NomiProcess_signals(list)
{
	var listen = _Runtime.sync(function() {
		var handle = { pending: [], waiting: [], ended: false, listeners: [] };
		__List_toArray(list).forEach(function(signal) {
			var name = _NomiProcess_name(signal);
			if (name === 'SIGKILL') return;
			var listener = function() {
				handle.pending.indexOf(signal) < 0 && handle.pending.push(signal);
				_NomiProcess_tell(handle);
			};
			try
			{
				process.on(name, listener);
				handle.listeners.push([name, listener]);
			}
			catch (e)
			{
				// A signal the system doesn't have never comes.
			}
		});
		return handle;
	});
	return _Runtime.acquireRelease(listen, function(handle) {
		return _Runtime.sync(function() {
			handle.listeners.forEach(function(listening) { process.off(listening[0], listening[1]); });
			handle.ended = true;
			_NomiProcess_tell(handle);
			return __Utils_Tuple0;
		});
	});
}

// A signal waits for another fiber's, not the host's: a program waiting
// for one holds no test's clock. While a fiber waits for one, the process
// stays alive, as listening alone wouldn't keep it, and a program waiting
// only for a signal isn't stuck.
function _NomiProcess_signal(handle)
{
	return _Runtime.wait(function(resume) {
		handle.waiting.push(resume);
		_NomiProcess_tell(handle);
		return function() {
			var at = handle.waiting.indexOf(resume);
			at < 0 || handle.waiting.splice(at, 1);
			_NomiProcess_alive(handle);
		};
	});
}

function _NomiProcess_tell(handle)
{
	while (handle.waiting.length > 0 && (handle.pending.length > 0 || handle.ended))
	{
		handle.waiting.shift()(_Runtime.succeed(handle.pending.length > 0 ? __Maybe_Just(handle.pending.shift()) : __Maybe_Nothing));
	}
	_NomiProcess_alive(handle);
}

// Keeps the process alive while a fiber waits for a signal.
function _NomiProcess_alive(handle)
{
	if (handle.waiting.length > 0 && !handle.ended)
	{
		handle.alive = handle.alive || setInterval(function() {}, 1 << 30);
	}
	else if (handle.alive)
	{
		clearInterval(handle.alive);
		handle.alive = null;
	}
}
