/*

import Elm.Kernel.List exposing (fromArray, toArray)
import Elm.Kernel.NomiStream exposing (reader, give, fail, wanting, bytes, view, writer, error)
import Elm.Kernel.Utils exposing (Tuple0)
import Maybe exposing (Just, Nothing)
import Nomi.Fs as Fs exposing (NotFound, PermissionDenied, AlreadyExists, IsADirectory, NotEmpty, NotUtf8, OutsideRoot, Other, File, Directory, Special)
import Time exposing (millisToPosix)

*/


// FILE SYSTEMS

// A file system capability: a store, which is a directory on disk or a
// map in memory; the directory in it every path starts from, as the names
// each `subdir` added, a list per `subdir`; and whether it may write. A
// `subdir` whose `..`s climbed out of its parent is `outside`, the path it
// was given, which every operation fails with.
function _NomiFs_make(store, root, writable, outside)
{
	return { store: store, root: root, writable: writable, outside: outside };
}

// The program's file system: the disk, from the working directory.
function _NomiFs_cwd()
{
	return _NomiFs_make(_NomiFs_disk(process.cwd()), [], true, null);
}

// A file system in memory holding `files`, a list of paths and their
// texts, with the directories they need.
function _NomiFs_memory(files)
{
	return A2(_NomiFs_copy, files, _NomiFs_make(_NomiFs_inMemory(), [], true, null));
}

// A new file system in memory holding what the one in memory `fs` holds
// and `files` too, written from the directory `fs` starts from, each new
// one modified at 0. A file the disk couldn't hold there, out of the root
// or under another file, makes the store `invalid`, and every operation
// on it a defect.
var _NomiFs_copy = F2(function(files, fs)
{
	var store = _NomiFs_inMemory();
	store.invalid = fs.store.invalid;
	fs.store.entries.forEach(function(entry, key) {
		store.entries.set(key, _NomiFs_clone(entry));
	});
	var root = [].concat.apply([], fs.root);
	__List_toArray(files).forEach(function(file) {
		var names = fs.outside === null ? _NomiFs_names(file.a) : null;
		var invalid = names === null ? ' is outside the root' : names.length === 0 ? ' names no file' : null;
		names = invalid === null ? root.concat(names) : [];
		for (var i = 1; i < names.length && invalid === null; i++)
		{
			var entry = store.entries.get(_NomiFs_key(names.slice(0, i)));
			if (entry === undefined) store.entries.set(_NomiFs_key(names.slice(0, i)), _NomiFs_directory(0));
			else if (entry.kind !== 'directory') invalid = ' is under a file';
		}
		var there = store.entries.get(_NomiFs_key(names));
		if (invalid === null && there !== undefined && there.kind === 'directory') invalid = ' is a directory';
		if (invalid !== null) store.invalid = store.invalid || 'withFiles: ' + JSON.stringify(file.a) + invalid;
		else store.entries.set(_NomiFs_key(names), _NomiFs_file(_NomiFs_encoder.encode(file.b), 0));
	});
	return _NomiFs_make(store, fs.root, fs.writable, fs.outside);
});

// Windows takes `\` for a separator too, so it separates names there.
var _NomiFs_separator = process.platform === 'win32' ? /[\\/]/ : '/';

// The names `path` leads to, or null when its `..`s climb out of where it
// starts. Empty names and `.` stay where they are.
function _NomiFs_names(path)
{
	var names = [];
	var parts = path.split(_NomiFs_separator);
	for (var i = 0; i < parts.length; i++)
	{
		var part = parts[i];
		if (part === '' || part === '.') continue;
		if (part !== '..') names.push(part);
		else if (names.pop() === undefined) return null;
	}
	return names;
}

var _NomiFs_subdir = F2(function(path, fs)
{
	if (fs.outside !== null) return fs;
	var names = _NomiFs_names(path);
	return names === null
		? _NomiFs_make(fs.store, fs.root, fs.writable, path)
		: _NomiFs_make(fs.store, fs.root.concat([names]), fs.writable, null);
});

function _NomiFs_readOnly(fs)
{
	return _NomiFs_make(fs.store, fs.root, false, fs.outside);
}

// A new, empty directory, as a capability of its own: on disk under the
// system's temporary directory, in memory a store of its own. It is
// removed with everything in it once the fiber that made it ends. Only a
// capability that may write makes one.
function _NomiFs_temporaryDirectory(fs)
{
	var named = 'temporary directory';
	if (fs.store.invalid) return _Runtime.die(new Error(fs.store.invalid));
	if (!fs.writable) return _Runtime.fail(__Fs_PermissionDenied(named));
	var made = _Runtime.onError(
		_Runtime.fromPromise(function() { return fs.store.temporary(); }),
		function(error) { return _Runtime.fail(_NomiFs_error(named, error)); }
	);
	var owned = _Runtime.acquireRelease(made, function(temporary) {
		return _Runtime.fromPromise(function() { return temporary.remove().catch(function() {}); });
	});
	return _Runtime.map(owned, function(temporary) {
		return _NomiFs_make(temporary.store, [], true, null);
	});
}


// OPERATIONS

var _NomiFs_encoder = new TextEncoder();

// Refuses what isn't UTF-8, and keeps a byte order mark, as the file has
// one.
var _NomiFs_decoder = new TextDecoder('utf-8', { fatal: true, ignoreBOM: true });

var _NomiFs_read = F2(function(path, fs)
{
	return _NomiFs_run(path, fs, _NomiFs_reads, function(store, names, signal) {
		return store.read(fs.root, names, signal).then(function(bytes) {
			try
			{
				return _NomiFs_decoder.decode(bytes);
			}
			catch (e)
			{
				throw _NomiFs_code('NOTUTF8');
			}
		});
	});
});

var _NomiFs_readBytes = F2(function(path, fs)
{
	return _NomiFs_run(path, fs, _NomiFs_reads, function(store, names, signal) {
		return store.read(fs.root, names, signal).then(__NomiStream_bytes);
	});
});

var _NomiFs_write = F3(function(path, text, fs)
{
	return _NomiFs_put(path, _NomiFs_encoder.encode(text), _NomiFs_truncate, fs);
});

var _NomiFs_writeBytes = F3(function(path, bytes, fs)
{
	return _NomiFs_put(path, __NomiStream_view(bytes), _NomiFs_truncate, fs);
});

var _NomiFs_append = F3(function(path, text, fs)
{
	return _NomiFs_put(path, _NomiFs_encoder.encode(text), _NomiFs_appending, fs);
});

// What a store does with a file already there, when it writes.
var _NomiFs_truncate = 'truncate';
var _NomiFs_appending = 'append';

function _NomiFs_put(path, bytes, mode, fs)
{
	return _NomiFs_run(path, fs, _NomiFs_writes, function(store, names, signal, now) {
		return store.write(fs.root, names, bytes, mode, signal, now).then(_NomiFs_unit);
	});
}

var _NomiFs_list = F2(function(path, fs)
{
	return _NomiFs_run(path, fs, _NomiFs_reads, function(store, names) {
		return store.list(fs.root, names).then(function(entries) {
			return __List_fromArray(entries.sort());
		});
	});
});

// The root itself is not removed: it is what the capability is.
var _NomiFs_remove = F3(function(options, path, fs)
{
	return _NomiFs_run(path, fs, _NomiFs_writes, function(store, names, signal, now) {
		return names.length === 0
			? Promise.reject(_NomiFs_code('EPERM'))
			: store.remove(fs.root, names, options.__$recursive, now).then(_NomiFs_unit);
	});
});

var _NomiFs_makeDirectory = F2(function(path, fs)
{
	return _NomiFs_run(path, fs, _NomiFs_writes, function(store, names, signal, now) {
		return store.makeDirectory(fs.root, names, now).then(_NomiFs_unit);
	});
});

// Whatever goes wrong, including a path out of the root, is `False`.
var _NomiFs_exists = F2(function(path, fs)
{
	if (fs.store.invalid) return _Runtime.die(new Error(fs.store.invalid));
	var names = fs.outside === null ? _NomiFs_names(path) : null;
	if (names === null) return _Runtime.succeed(false);
	var exists = _Runtime.fromPromise(function() { return fs.store.exists(fs.root, names); });
	return _Runtime.onError(exists, function() { return _Runtime.succeed(false); });
});

var _NomiFs_stat = F2(function(path, fs)
{
	return _NomiFs_run(path, fs, _NomiFs_reads, function(store, names) {
		return store.stat(fs.root, names).then(function(stat) {
			return {
				__$kind: stat.kind === 'file' ? __Fs_File : stat.kind === 'directory' ? __Fs_Directory : __Fs_Special,
				__$size: stat.size,
				__$modified: __Time_millisToPosix(Math.floor(stat.modified))
			};
		});
	});
});

// `to`, checked as `path` is, beside it: the error names whichever failed.
var _NomiFs_rename = F3(function(from, to, fs)
{
	return _NomiFs_both(from, to, fs, function(store, names, other, now) {
		return store.rename(fs.root, names, other, now);
	});
});

var _NomiFs_copyPath = F3(function(from, to, fs)
{
	return _NomiFs_both(from, to, fs, function(store, names, other, now) {
		return store.copy(fs.root, names, other, now);
	});
});

// A link's target is a path from the link's directory, which mustn't
// lead out of the capability, as the link's own path mustn't.
var _NomiFs_symlink = F3(function(target, path, fs)
{
	return _NomiFs_run(path, fs, _NomiFs_writes, function(store, names, signal, now) {
		var from = names.slice(0, -1).join('/');
		if (names.length === 0 || target.charAt(0) === '/' || _NomiFs_names(from + '/' + target) === null)
		{
			return Promise.reject(_NomiFs_code('OUTSIDE'));
		}
		return store.symlink(fs.root, target, names, now).then(_NomiFs_unit);
	});
});

var _NomiFs_readLink = F2(function(path, fs)
{
	return _NomiFs_run(path, fs, _NomiFs_reads, function(store, names) {
		return store.readLink(fs.root, names);
	});
});

// The path with every symbolic link in it followed, from the capability's
// root, which a capability never shows beyond.
var _NomiFs_realPath = F2(function(path, fs)
{
	return _NomiFs_run(path, fs, _NomiFs_reads, function(store, names) {
		return store.realPath(fs.root, names);
	});
});

// The file as a `Blob`, which a server sends with its size: on disk Bun's
// file at the path the capability resolved, opened as it is sent; in
// memory its bytes as they are now.
var _NomiFs_blob = F2(function(path, fs)
{
	return _NomiFs_run(path, fs, _NomiFs_reads, function(store, names) {
		return store.blob(fs.root, names);
	});
});

// Where the file at the path is on the disk, for what opens a file by its
// path itself, as SQLite does: its real path and whether `fs` may write,
// or null where `fs` has no disk. Where it may write, the file may not be
// there yet, but the directory it goes in must.
var _NomiFs_onDisk = F2(function(path, fs)
{
	return _NomiFs_run(path, fs, _NomiFs_reads, function(store, names) {
		return store.onDisk ? store.onDisk(fs.root, names, fs.writable) : Promise.resolve(null);
	});
});

var _NomiFs_reads = false;
var _NomiFs_writes = true;

function _NomiFs_unit()
{
	return __Utils_Tuple0;
}

// The operation `f` on `fs` at `path`, given the store, the names `path`
// leads to, an abort signal and the time on the program's clock, as a
// task failing with an `Fs.Error` that names `path`.
function _NomiFs_run(path, fs, writes, f)
{
	if (fs.store.invalid) return _Runtime.die(new Error(fs.store.invalid));
	if (fs.outside !== null) return _Runtime.fail(__Fs_OutsideRoot(fs.outside));
	var names = _NomiFs_names(path);
	if (names === null) return _Runtime.fail(__Fs_OutsideRoot(path));
	if (writes && !fs.writable) return _Runtime.fail(__Fs_PermissionDenied(path));
	var run = _Runtime.andThen(_Runtime.now, function(now) {
		return _Runtime.fromPromise(function(signal) { return f(fs.store, names, signal, now); });
	});
	return _Runtime.onError(run, function(error) {
		return _Runtime.fail(_NomiFs_error(path, error));
	});
}

// `f` on the paths `from` and `to`, both written to, failing with the
// error of the path that failed: a store's error carries `to` when it is
// that path's.
function _NomiFs_both(from, to, fs, f)
{
	var names = fs.outside === null ? _NomiFs_names(to) : null;
	if (fs.outside === null && names === null) return _Runtime.fail(__Fs_OutsideRoot(to));
	if (fs.outside === null && names.length === 0) return _Runtime.fail(__Fs_PermissionDenied(to));
	return _NomiFs_run(from, fs, _NomiFs_writes, function(store, from, signal, now) {
		if (from.length === 0) return Promise.reject(_NomiFs_code('EPERM'));
		return f(store, from, names, now).then(_NomiFs_unit, function(error) {
			if (error.to) error.named = to;
			throw error;
		});
	});
}

// What a store's error means for `path`. `node:fs`'s errors and the memory
// store's carry the same codes; `OUTSIDE` is a symbolic link's way out.
function _NomiFs_error(path, error)
{
	path = error.named !== undefined ? error.named : path;
	switch (error.code)
	{
		case 'ENOENT':
			return __Fs_NotFound(path);
		case 'EACCES':
		case 'EPERM':
		case 'EROFS':
			return __Fs_PermissionDenied(path);
		case 'EEXIST':
		case 'ERR_FS_CP_EEXIST':
			return __Fs_AlreadyExists(path);
		case 'EISDIR':
		case 'ERR_FS_EISDIR':
			return __Fs_IsADirectory(path);
		case 'ENOTEMPTY':
			return __Fs_NotEmpty(path);
		case 'NOTUTF8':
			return __Fs_NotUtf8(path);
		case 'OUTSIDE':
			return __Fs_OutsideRoot(path);
		case 'ENOTDIR':
			return __Fs_Other(path + ': not a directory');
		case 'EINVAL':
		case 'ERR_FS_CP_EINVAL':
			return __Fs_Other(path + (error.syscall === 'readlink' || error.link ? ': not a symbolic link' : ': would go inside itself'));
		default:
			return __Fs_Other(path + ': ' + error.message);
	}
}

function _NomiFs_code(code)
{
	var error = new Error(code);
	error.code = code;
	return error;
}


// FILES AS STREAMS

// The file's bytes as a reader, which the fiber that opened it owns: the
// file is closed at its end, or once that fiber ends.
var _NomiFs_reader = F2(function(path, fs)
{
	var opened = _NomiFs_run(path, fs, _NomiFs_reads, function(store, names) {
		return store.openRead(fs.root, names);
	});
	return _Runtime.map(_NomiFs_owned(opened), function(file) {
		var reading = false;
		return __NomiStream_reader(fs.store.host, function(reader) {
			if (reading || !__NomiStream_wanting(reader)) return;
			reading = true;
			file.next().then(function(chunk) {
				reading = false;
				chunk === null && file.close();
				__NomiStream_give(reader, chunk === null ? null : __NomiStream_bytes(chunk));
			}, function(error) {
				reading = false;
				file.close();
				__NomiStream_fail(reader, { error: __NomiStream_error(error) });
			});
		});
	});
});

var _NomiFs_writer = F2(function(path, fs)
{
	return _NomiFs_openWriter(path, fs, _NomiFs_truncate);
});

var _NomiFs_appender = F2(function(path, fs)
{
	return _NomiFs_openWriter(path, fs, _NomiFs_appending);
});

// A writer to the file, which the fiber that opened it owns: closing it,
// or that fiber ending, closes the file.
function _NomiFs_openWriter(path, fs, mode)
{
	var opened = _NomiFs_run(path, fs, _NomiFs_writes, function(store, names, signal, now) {
		return store.openWrite(fs.root, names, mode, now);
	});
	var done = function(promise) {
		return _Runtime.onError(_Runtime.fromPromise(promise), function(error) {
			return _Runtime.fail(__NomiStream_error(error));
		});
	};
	return _Runtime.map(_NomiFs_owned(opened), function(file) {
		return __NomiStream_writer({
			write: function(bytes) {
				return _Runtime.map(done(function() { return file.write(__NomiStream_view(bytes)); }), _NomiFs_unit);
			},
			close: _Runtime.map(done(function() { return file.close(); }), _NomiFs_unit),
			terminal: false,
			size: function() { return null; }
		});
	});
}

// What `opened` gives, closed once the fiber that opened it ends: that
// fiber ends once the file has closed, so whoever waits for it finds the
// file closed.
function _NomiFs_owned(opened)
{
	return _Runtime.acquireRelease(opened, function(file) {
		return _Runtime.fromPromise(function() {
			return Promise.resolve(file.close()).then(_NomiFs_unit, _NomiFs_unit);
		});
	});
}


// WATCHING

// A watch: the paths that changed since the last `change` took them, from
// the watched path, or whether too many did to keep each; how it ended,
// once it did; the fibers waiting for a change, oldest first; and whether
// they wait for the host. It is the fiber's that made it, and stops once
// that fiber ends.
var _NomiFs_watch = F2(function(path, fs)
{
	var watch = { paths: new Set(), overflow: false, end: null, waiting: [], host: fs.store.host };
	var opened = _NomiFs_run(path, fs, _NomiFs_reads, function(store, names) {
		return store.watch(fs.root, names, function(changed) {
			watch.paths.size < _NomiFs_mostChanges ? watch.paths.add(changed) : (watch.overflow = true);
			_NomiFs_tell(watch);
		}, function(error) {
			watch.end = watch.end || { error: __Fs_Other(path + ': ' + (error && error.message || error)) };
			_NomiFs_tell(watch);
		});
	});
	// The watch ends once its watcher closes, as its fiber ends.
	var ending = _Runtime.map(opened, function(watcher) {
		var close = watcher.close;
		watcher.close = function() {
			close.call(watcher);
			watch.end = watch.end || {};
			_NomiFs_tell(watch);
		};
		return watcher;
	});
	return _Runtime.map(_NomiFs_owned(ending), function() {
		return watch;
	});
});

// How many paths a watch keeps apart before it says only that something
// under it changed.
var _NomiFs_mostChanges = 1000;

function _NomiFs_change(watch)
{
	var wait = watch.host ? _Runtime.waitHost : _Runtime.wait;
	return wait(function(resume) {
		watch.waiting.push(resume);
		_NomiFs_tell(watch);
		return function() {
			var at = watch.waiting.indexOf(resume);
			at < 0 || watch.waiting.splice(at, 1);
		};
	});
}

// Hands the paths that changed to the first fiber waiting, or the end.
function _NomiFs_tell(watch)
{
	while (watch.waiting.length > 0 && (watch.paths.size > 0 || watch.overflow || watch.end !== null))
	{
		var resume = watch.waiting.shift();
		if (watch.paths.size > 0 || watch.overflow)
		{
			var paths = watch.overflow ? ['.'] : Array.from(watch.paths).sort();
			watch.paths.clear();
			watch.overflow = false;
			resume(_Runtime.succeed(__Maybe_Just({ __$paths: __List_fromArray(paths) })));
		}
		else
		{
			resume(watch.end.error !== undefined ? _Runtime.fail(watch.end.error) : _Runtime.succeed(__Maybe_Nothing));
		}
	}
}


// RESOLVING

// The location `names` leads to from `root`, in `space`: the real
// location of the longest part of it that exists, followed by the names
// that don't, or by the last name when `follow` is false. That part must
// be inside the root, and each subdirectory the root is made of inside
// the one before it, from the space's base. A root that doesn't exist yet
// is names that don't, so `makeDirectory` makes it.
//
// A space is the disk's, whose locations are real paths, or a memory
// store's, whose locations are lists of names: `base()` is the real
// location of its base, `realpath(at, names)` the real location below
// `at`, failing as the disk does, `join(at, names)` a location below `at`,
// and `inside(dir, at)` whether one real location is in another.
function _NomiFs_resolve(space, root, names, follow)
{
	var within = function(dir, below)
	{
		return space.realpath(dir, below).then(function(real) {
			if (!space.inside(dir, real)) throw _NomiFs_code('OUTSIDE');
			return real;
		});
	};
	// `at`, a real location and the names past it that don't exist,
	// followed down `below`: once a name is missing, every one after it is
	// too.
	var existing = function(at, below)
	{
		if (at.missing.length > 0) return Promise.resolve({ real: at.real, missing: at.missing.concat(below) });
		var longest = function(n)
		{
			if (n === 0) return Promise.resolve({ real: at.real, missing: below });
			return within(at.real, below.slice(0, n)).then(function(real) {
				return { real: real, missing: below.slice(n) };
			}, function(error) {
				if (error.code !== 'ENOENT' && error.code !== 'ENOTDIR') throw error;
				return longest(n - 1);
			});
		};
		return longest(below.length);
	};
	var last = follow ? [] : names.slice(-1);
	var resolved = root.concat([follow ? names : names.slice(0, -1)]).reduce(function(at, below) {
		return at.then(function(at) { return existing(at, below); });
	}, space.base().then(function(real) { return { real: real, missing: [] }; }));
	return resolved.then(function(at) {
		return space.join(at.real, at.missing.concat(last));
	});
}

// The real path from the capability's root to `names`, which must exist,
// `/`-separated, or `.` for the root itself.
function _NomiFs_realFrom(space, root, names, exists)
{
	return Promise.all([
		_NomiFs_resolve(space, root, [], true),
		_NomiFs_resolve(space, root, names, true).then(function(at) {
			return exists(at).then(function() { return at; });
		})
	]).then(function(both) {
		return space.relative(both[0], both[1]);
	});
}


// ON DISK

var _NomiFs_fs = process.getBuiltinModule('node:fs');
var _NomiFs_promises = _NomiFs_fs.promises;
var _NomiFs_path = process.getBuiltinModule('node:path');
var _NomiFs_os = process.getBuiltinModule('node:os');

// Opening a symbolic link with it fails, where the platform has it.
var _NomiFs_noFollow = _NomiFs_fs.constants.O_NOFOLLOW || 0;

// How much of a file a read of its stream takes.
var _NomiFs_chunk = 64 * 1024;

// The disk under the directory `base`. A path is resolved through its
// symbolic links before each operation (`_NomiFs_resolve`), and one that
// leads out of the capability's root, or out of `base`, is `OUTSIDE`;
// files are then opened with `O_NOFOLLOW`, so a last name that has become
// a symbolic link since is refused too. A directory swapped for a symbolic
// link between the two can still be followed: containment holds against
// the tree as it is when the operation starts, not against another process
// changing it meanwhile.
function _NomiFs_disk(base)
{
	var p = _NomiFs_promises;
	var path = _NomiFs_path;
	var flags = _NomiFs_fs.constants;
	var real = null;
	var space = {
		base: function() { return real = real || p.realpath(base); },
		realpath: function(dir, below) { return p.realpath(path.join.apply(null, [dir].concat(below))); },
		join: function(at, names) { return path.join.apply(null, [at].concat(names)); },
		inside: _NomiFs_inside,
		relative: function(dir, at) { return path.relative(dir, at).split(path.sep).join('/') || '.'; }
	};
	var resolve = function(root, names, follow) { return _NomiFs_resolve(space, root, names, follow); };
	// That the directory `at` goes in is one.
	var parentOf = function(at)
	{
		return p.stat(path.dirname(at)).then(function(stat) {
			if (!stat.isDirectory()) throw _NomiFs_code('ENOTDIR');
		});
	};
	var writeFlags = function(mode) {
		return flags.O_WRONLY | flags.O_CREAT | (mode === _NomiFs_appending ? flags.O_APPEND : flags.O_TRUNC) | _NomiFs_noFollow;
	};
	return {
		host: true,
		read: function(root, names, signal) {
			return resolve(root, names, true).then(function(at) {
				return _NomiFs_open(p.readFile(at, { flag: flags.O_RDONLY | _NomiFs_noFollow, signal: signal }));
			});
		},
		write: function(root, names, bytes, mode, signal) {
			return resolve(root, names, true).then(function(at) {
				return _NomiFs_open(p.writeFile(at, bytes, { flag: writeFlags(mode), signal: signal }));
			});
		},
		list: function(root, names) {
			return resolve(root, names, true).then(function(at) {
				return p.readdir(at);
			});
		},
		// The last name is not followed, so a symbolic link is removed
		// rather than what it leads to.
		remove: function(root, names, recursive) {
			return resolve(root, names, false).then(function(at) {
				return recursive
					? p.rm(at, { recursive: true })
					: p.lstat(at).then(function(stat) { return stat.isDirectory() ? p.rmdir(at) : p.unlink(at); });
			});
		},
		makeDirectory: function(root, names) {
			return resolve(root, names, true).then(function(at) {
				return p.mkdir(at, { recursive: true });
			});
		},
		exists: function(root, names) {
			return resolve(root, names, true).then(function(at) {
				return p.stat(at);
			}).then(function() { return true; });
		},
		stat: function(root, names) {
			return resolve(root, names, true).then(function(at) {
				return p.stat(at);
			}).then(function(stat) {
				return {
					kind: stat.isFile() ? 'file' : stat.isDirectory() ? 'directory' : 'special',
					size: stat.isFile() ? stat.size : 0,
					modified: stat.mtimeMs
				};
			});
		},
		// Errors about where things go are the second path's, but for one
		// going inside itself.
		rename: function(root, from, to) {
			return Promise.all([resolve(root, from, false), _NomiFs_to(resolve(root, to, false))]).then(function(both) {
				return p.lstat(both[0]).then(function() {
					return _NomiFs_to(parentOf(both[1]));
				}).then(function() {
					return p.rename(both[0], both[1]).catch(function(error) {
						if (error.code !== 'EINVAL') error.to = true;
						throw error;
					});
				});
			});
		},
		copy: function(root, from, to) {
			return Promise.all([resolve(root, from, true), _NomiFs_to(resolve(root, to, false))]).then(function(both) {
				return p.lstat(both[0]).then(function() {
					if (_NomiFs_inside(both[0], both[1])) throw _NomiFs_code('EINVAL');
					return _NomiFs_to(parentOf(both[1]).then(function() {
						return p.lstat(both[1]).then(function() {
							throw _NomiFs_code('EEXIST');
						}, function(error) {
							if (error.code !== 'ENOENT') throw error;
						});
					}));
				}).then(function() {
					return p.cp(both[0], both[1], { recursive: true, verbatimSymlinks: true, errorOnExist: true, force: false });
				});
			});
		},
		symlink: function(root, target, names) {
			return resolve(root, names, false).then(function(at) {
				return p.symlink(target, at);
			});
		},
		readLink: function(root, names) {
			return resolve(root, names, false).then(function(at) {
				return p.readlink(at);
			});
		},
		realPath: function(root, names) {
			return _NomiFs_realFrom(space, root, names, function(at) { return p.stat(at); });
		},
		blob: function(root, names) {
			return resolve(root, names, true).then(function(at) {
				return p.lstat(at).then(function(stat) {
					if (stat.isSymbolicLink()) throw _NomiFs_code('OUTSIDE');
					if (stat.isDirectory()) throw _NomiFs_code('EISDIR');
					return Bun.file(at);
				});
			});
		},
		onDisk: function(root, names, writable) {
			return resolve(root, names, true).then(function(at) {
				var found = { path: at, writable: writable };
				return p.stat(at).then(function(stat) {
					if (stat.isDirectory()) throw _NomiFs_code('EISDIR');
					return found;
				}, function(error) {
					if (error.code !== 'ENOENT' || !writable) throw error;
					return parentOf(at).then(function() { return found; });
				});
			});
		},
		openRead: function(root, names) {
			return resolve(root, names, true).then(function(at) {
				return _NomiFs_open(p.open(at, flags.O_RDONLY | _NomiFs_noFollow));
			}).then(function(handle) {
				// A directory opens for reading, but has no bytes to read.
				return handle.stat().then(function(stat) {
					if (!stat.isDirectory()) return handle;
					return handle.close().then(function() { throw _NomiFs_code('EISDIR'); });
				});
			}).then(function(handle) {
				return _NomiFs_handle(handle, function() {
					var buffer = new Uint8Array(_NomiFs_chunk);
					return handle.read(buffer, 0, buffer.length, null).then(function(read) {
						return read.bytesRead === 0 ? null : buffer.subarray(0, read.bytesRead);
					});
				});
			});
		},
		openWrite: function(root, names, mode) {
			return resolve(root, names, true).then(function(at) {
				return _NomiFs_open(p.open(at, writeFlags(mode), 438));
			}).then(function(handle) {
				return _NomiFs_handle(handle, null);
			});
		},
		watch: function(root, names, changed, failed) {
			return resolve(root, names, true).then(function(at) {
				return p.stat(at).then(function() {
					var watcher = _NomiFs_fs.watch(at, { recursive: true }, function(event, name) {
						changed(name ? String(name).split(path.sep).join('/') : '.');
					});
					watcher.on('error', failed);
					return {
						closed: false,
						close: function() {
							if (this.closed) return;
							this.closed = true;
							watcher.close();
						}
					};
				});
			});
		},
		temporary: function() {
			return p.mkdtemp(path.join(_NomiFs_os.tmpdir(), 'nomi-')).then(function(dir) {
				return {
					store: _NomiFs_disk(dir),
					remove: function() { return p.rm(dir, { recursive: true, force: true }); }
				};
			});
		}
	};
}

// A rejection of `promise` is the second path's.
function _NomiFs_to(promise)
{
	return promise.catch(function(error) {
		throw Object.assign(error, { to: true });
	});
}

// An open file: `next()` its next chunk, or null at its end, `write`
// all of the bytes, and `close`, which a second time does nothing.
function _NomiFs_handle(handle, next)
{
	return {
		closed: false,
		next: next,
		write: function(bytes) {
			var rest = function(from) {
				return from >= bytes.length ? undefined : handle.write(bytes, from, bytes.length - from, null).then(function(wrote) {
					return rest(from + wrote.bytesWritten);
				});
			};
			return Promise.resolve(rest(0));
		},
		close: function() {
			if (this.closed) return Promise.resolve();
			this.closed = true;
			return handle.close();
		}
	};
}

// Whether `path` is `dir` or inside it; both are real paths.
function _NomiFs_inside(dir, path)
{
	var relative = _NomiFs_path.relative(dir, path);
	return relative === '' || !(relative === '..' || relative.startsWith('..' + _NomiFs_path.sep) || _NomiFs_path.isAbsolute(relative));
}

// `O_NOFOLLOW` refusing a symbolic link is a way out of the root.
function _NomiFs_open(opened)
{
	return opened.catch(function(error) {
		throw error.code === 'ELOOP' ? _NomiFs_code('OUTSIDE') : error;
	});
}


// IN MEMORY

// What the memory store holds at a name: a directory, a file's bytes, or
// a symbolic link's target, each with when it was last modified.
function _NomiFs_directory(modified)
{
	return { kind: 'directory', modified: modified };
}

function _NomiFs_file(bytes, modified)
{
	return { kind: 'file', bytes: bytes, modified: modified };
}

function _NomiFs_clone(entry)
{
	return entry.kind === 'file' ? _NomiFs_file(entry.bytes.slice(), entry.modified) : Object.assign({}, entry);
}

function _NomiFs_key(names)
{
	return names.join('/');
}

// How many symbolic links a path may pass through, as the disk allows.
var _NomiFs_mostLinks = 40;

// Entries by their names joined with `/`, the root's being empty, and the
// watches on them. Each operation fails with the code the disk would give,
// and a symbolic link is followed as the disk follows one, never above
// the store's root.
function _NomiFs_inMemory()
{
	var entries = new Map([['', _NomiFs_directory(0)]]);
	var watchers = new Set();

	// The real names `names` lead to, every link in them followed.
	var canonical = function(names)
	{
		var real = [];
		var rest = names.slice();
		for (var hops = 0; rest.length > 0; )
		{
			var name = rest.shift();
			if (name === '' || name === '.') continue;
			if (name === '..')
			{
				if (real.pop() === undefined) throw _NomiFs_code('OUTSIDE');
				continue;
			}
			var entry = entries.get(_NomiFs_key(real.concat([name])));
			if (entry === undefined) throw _NomiFs_code('ENOENT');
			if (entry.kind === 'link')
			{
				if (++hops > _NomiFs_mostLinks) throw _NomiFs_code('ELOOP');
				if (entry.target.charAt(0) === '/') throw _NomiFs_code('OUTSIDE');
				rest = entry.target.split('/').concat(rest);
				continue;
			}
			if (entry.kind !== 'directory' && rest.length > 0) throw _NomiFs_code('ENOTDIR');
			real.push(name);
		}
		return real;
	};
	var space = {
		base: function() { return Promise.resolve([]); },
		realpath: function(dir, below) {
			return new Promise(function(resolve) { resolve(canonical(dir.concat(below))); });
		},
		join: function(at, names) { return at.concat(names); },
		inside: function(dir, at) {
			return dir.length <= at.length && dir.every(function(name, i) { return name === at[i]; });
		},
		relative: function(dir, at) { return at.slice(dir.length).join('/') || '.'; }
	};
	var resolve = function(root, names, follow) { return _NomiFs_resolve(space, root, names, follow); };

	// The entry at `at`, or undefined; each name before the last must be a
	// directory, and a link among them leads nowhere the disk would go.
	var lookup = function(at)
	{
		for (var i = 0; i < at.length; i++)
		{
			var entry = entries.get(_NomiFs_key(at.slice(0, i)));
			if (entry === undefined || entry.kind === 'link') throw _NomiFs_code('ENOENT');
			if (entry.kind !== 'directory') throw _NomiFs_code('ENOTDIR');
		}
		return entries.get(_NomiFs_key(at));
	};
	// The file at `at`, which a symbolic link still there leads out of.
	var file = function(at)
	{
		var entry = lookup(at);
		if (entry === undefined) throw _NomiFs_code('ENOENT');
		if (entry.kind === 'directory') throw _NomiFs_code('EISDIR');
		if (entry.kind === 'link') throw _NomiFs_code('OUTSIDE');
		return entry;
	};
	var put = function(at, bytes, mode, now)
	{
		var entry = lookup(at);
		if (entry !== undefined && entry.kind === 'directory') throw _NomiFs_code('EISDIR');
		if (entry !== undefined && entry.kind === 'link') throw _NomiFs_code('OUTSIDE');
		var append = mode === _NomiFs_appending && entry !== undefined;
		var bytesNow = new Uint8Array(append ? entry.bytes.length + bytes.length : bytes.length);
		append ? (bytesNow.set(entry.bytes), bytesNow.set(bytes, entry.bytes.length)) : bytesNow.set(bytes);
		entries.set(_NomiFs_key(at), _NomiFs_file(bytesNow, now));
		changed(at);
	};
	// The keys at and under `at`.
	var under = function(at)
	{
		var key = _NomiFs_key(at);
		var keys = [];
		entries.forEach(function(_, other) {
			if (other === key || (other.startsWith(key + '/') && key !== '') || (key === '' && other !== '')) keys.push(other);
		});
		return keys;
	};
	var changed = function(at)
	{
		watchers.forEach(function(watcher) {
			if (space.inside(watcher.at, at)) watcher.changed(space.relative(watcher.at, at));
		});
	};
	// Runs `f` once the locations resolve; what it throws rejects.
	var after = function(locations, f)
	{
		return Promise.all(locations).then(function(at) { return f.apply(null, at); });
	};
	return {
		host: false,
		entries: entries,
		read: function(root, names) {
			return after([resolve(root, names, true)], function(at) {
				return file(at).bytes.slice();
			});
		},
		write: function(root, names, bytes, mode, signal, now) {
			return after([resolve(root, names, true)], function(at) {
				put(at, bytes, mode, now);
			});
		},
		list: function(root, names) {
			return after([resolve(root, names, true)], function(at) {
				var entry = lookup(at);
				if (entry === undefined) throw _NomiFs_code('ENOENT');
				if (entry.kind !== 'directory') throw _NomiFs_code('ENOTDIR');
				var prefix = at.length === 0 ? '' : _NomiFs_key(at) + '/';
				var children = [];
				entries.forEach(function(_, key) {
					if (key !== '' && key.startsWith(prefix) && key.indexOf('/', prefix.length) < 0) children.push(key.slice(prefix.length));
				});
				return children;
			});
		},
		remove: function(root, names, recursive) {
			return after([resolve(root, names, false)], function(at) {
				var entry = lookup(at);
				if (entry === undefined) throw _NomiFs_code('ENOENT');
				var keys = under(at);
				if (!recursive && keys.length > 1) throw _NomiFs_code('ENOTEMPTY');
				keys.forEach(function(key) { entries.delete(key); });
				changed(at);
			});
		},
		makeDirectory: function(root, names, now) {
			return after([resolve(root, names, true)], function(at) {
				for (var i = 1; i <= at.length; i++)
				{
					var key = _NomiFs_key(at.slice(0, i));
					var entry = entries.get(key);
					if (entry === undefined)
					{
						entries.set(key, _NomiFs_directory(now));
						changed(at.slice(0, i));
					}
					else if (entry.kind === 'link') throw _NomiFs_code(i === at.length ? 'EEXIST' : 'ENOENT');
					else if (entry.kind !== 'directory') throw _NomiFs_code(i === at.length ? 'EEXIST' : 'ENOTDIR');
				}
			});
		},
		exists: function(root, names) {
			return after([resolve(root, names, true)], function(at) {
				var entry = lookup(at);
				return entry !== undefined && entry.kind !== 'link';
			});
		},
		stat: function(root, names) {
			return after([resolve(root, names, true)], function(at) {
				var entry = lookup(at);
				if (entry === undefined || entry.kind === 'link') throw _NomiFs_code('ENOENT');
				return { kind: entry.kind, size: entry.kind === 'file' ? entry.bytes.length : 0, modified: entry.modified };
			});
		},
		rename: function(root, from, to, now) {
			return after([resolve(root, from, false), _NomiFs_to(resolve(root, to, false))], function(from, at) {
				var entry = lookup(from);
				if (entry === undefined) throw _NomiFs_code('ENOENT');
				var there = _NomiFs_onTo(function() { return lookup(at); });
				if (space.inside(from, at) && from.length < at.length) throw _NomiFs_code('EINVAL');
				if (_NomiFs_key(from) === _NomiFs_key(at)) return;
				if (there !== undefined)
				{
					if (there.kind === 'directory' && entry.kind !== 'directory') throw Object.assign(_NomiFs_code('EISDIR'), { to: true });
					if (there.kind !== 'directory' && entry.kind === 'directory') throw Object.assign(_NomiFs_code('ENOTDIR'), { to: true });
					if (there.kind === 'directory' && under(at).length > 1) throw Object.assign(_NomiFs_code('ENOTEMPTY'), { to: true });
					under(at).forEach(function(key) { entries.delete(key); });
				}
				_NomiFs_move(entries, under(from), from.length, at);
				changed(from);
				changed(at);
			});
		},
		copy: function(root, from, to, now) {
			return after([resolve(root, from, true), _NomiFs_to(resolve(root, to, false))], function(from, at) {
				var entry = lookup(from);
				if (entry === undefined || entry.kind === 'link') throw _NomiFs_code('ENOENT');
				if (space.inside(from, at)) throw _NomiFs_code('EINVAL');
				if (_NomiFs_onTo(function() { return lookup(at); }) !== undefined) throw Object.assign(_NomiFs_code('EEXIST'), { to: true });
				var copies = under(from).map(function(key) {
					var copy = _NomiFs_clone(entries.get(key));
					copy.modified = now;
					return [at.concat(key.split('/').slice(from.length)).join('/'), copy];
				});
				copies.forEach(function(copy) { entries.set(copy[0], copy[1]); });
				changed(at);
			});
		},
		symlink: function(root, target, names, now) {
			return after([resolve(root, names, false)], function(at) {
				if (lookup(at) !== undefined) throw _NomiFs_code('EEXIST');
				entries.set(_NomiFs_key(at), { kind: 'link', target: target, modified: now });
				changed(at);
			});
		},
		readLink: function(root, names) {
			return after([resolve(root, names, false)], function(at) {
				var entry = lookup(at);
				if (entry === undefined) throw _NomiFs_code('ENOENT');
				if (entry.kind !== 'link') throw Object.assign(_NomiFs_code('EINVAL'), { link: true });
				return entry.target;
			});
		},
		realPath: function(root, names) {
			return _NomiFs_realFrom(space, root, names, function(at) {
				return new Promise(function(resolve) {
					var entry = lookup(at);
					if (entry === undefined || entry.kind === 'link') throw _NomiFs_code('ENOENT');
					resolve();
				});
			});
		},
		blob: function(root, names) {
			return after([resolve(root, names, true)], function(at) {
				return new Blob([file(at).bytes.slice()]);
			});
		},
		// A reader of the file as it is when it is opened.
		openRead: function(root, names) {
			return after([resolve(root, names, true)], function(at) {
				var bytes = file(at).bytes.slice();
				return {
					next: function() {
						var chunk = bytes;
						bytes = null;
						return Promise.resolve(chunk);
					},
					close: function() {}
				};
			});
		},
		openWrite: function(root, names, mode, now) {
			return after([resolve(root, names, true)], function(at) {
				put(at, new Uint8Array(0), mode, now);
				return {
					write: function(bytes) {
						return new Promise(function(resolve) {
							var entry = entries.get(_NomiFs_key(at));
							put(at, bytes, _NomiFs_appending, entry === undefined ? now : entry.modified);
							resolve();
						});
					},
					close: function() { return Promise.resolve(); }
				};
			});
		},
		watch: function(root, names, onChange) {
			return after([resolve(root, names, true)], function(at) {
				var entry = lookup(at);
				if (entry === undefined || entry.kind === 'link') throw _NomiFs_code('ENOENT');
				var watcher = { at: at, changed: onChange };
				watchers.add(watcher);
				return {
					closed: false,
					close: function() {
						this.closed = true;
						watchers.delete(watcher);
					}
				};
			});
		},
		temporary: function() {
			return Promise.resolve({ store: _NomiFs_inMemory(), remove: function() { return Promise.resolve(); } });
		}
	};
}

// What `f` gives; what it throws is the second path's.
function _NomiFs_onTo(f)
{
	try
	{
		return f();
	}
	catch (error)
	{
		throw Object.assign(error, { to: true });
	}
}

// Moves the entries at `keys`, whose first `depth` names are the ones
// moved, to under `at`.
function _NomiFs_move(entries, keys, depth, at)
{
	var moved = keys.map(function(key) {
		var entry = entries.get(key);
		entries.delete(key);
		return [at.concat(key.split('/').slice(depth)).join('/'), entry];
	});
	moved.forEach(function(entry) { entries.set(entry[0], entry[1]); });
}
