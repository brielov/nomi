/*

import Elm.Kernel.List exposing (fromArray, toArray)
import Elm.Kernel.Utils exposing (Tuple0)
import Nomi.Fs as Fs exposing (NotFound, PermissionDenied, AlreadyExists, IsADirectory, OutsideRoot, Other)

*/


// FILE SYSTEMS

// A file system capability: a store, which is a directory on disk or a
// map in memory; the directory in it every path starts from, as the names
// each `subdir` added, a list per `subdir`; and whether it may write. A `subdir` whose `..`s climbed out of
// its parent is `outside`, the path it was given, which every operation
// fails with.
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
// and `files` too, written from the directory `fs` starts from.
var _NomiFs_copy = F2(function(files, fs)
{
	var store = _NomiFs_inMemory();
	fs.store.entries.forEach(function(entry, key) {
		store.entries.set(key, entry === _NomiFs_directory ? entry : entry.slice());
	});
	var root = [].concat.apply([], fs.root);
	__List_toArray(files).forEach(function(file) {
		var names = _NomiFs_names(file.a);
		if (fs.outside !== null || names === null || names.length === 0) return;
		names = root.concat(names);
		for (var i = 1; i < names.length; i++)
		{
			store.entries.has(_NomiFs_key(names.slice(0, i))) || store.entries.set(_NomiFs_key(names.slice(0, i)), _NomiFs_directory);
		}
		store.entries.set(_NomiFs_key(names), _NomiFs_encoder.encode(file.b));
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


// OPERATIONS

var _NomiFs_encoder = new TextEncoder();

// Keeps a byte order mark, as the file has one.
var _NomiFs_decoder = new TextDecoder('utf-8', { ignoreBOM: true });

var _NomiFs_read = F2(function(path, fs)
{
	return _NomiFs_run(path, fs, _NomiFs_reads, function(store, names, signal) {
		return store.read(fs.root, names, signal).then(function(bytes) {
			return _NomiFs_decoder.decode(bytes);
		});
	});
});

var _NomiFs_readBytes = F2(function(path, fs)
{
	return _NomiFs_run(path, fs, _NomiFs_reads, function(store, names, signal) {
		return store.read(fs.root, names, signal).then(function(bytes) {
			return new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
		});
	});
});

var _NomiFs_write = F3(function(path, text, fs)
{
	return _NomiFs_run(path, fs, _NomiFs_writes, function(store, names, signal) {
		return store.write(fs.root, names, _NomiFs_encoder.encode(text), false, signal).then(_NomiFs_unit);
	});
});

var _NomiFs_append = F3(function(path, text, fs)
{
	return _NomiFs_run(path, fs, _NomiFs_writes, function(store, names, signal) {
		return store.write(fs.root, names, _NomiFs_encoder.encode(text), true, signal).then(_NomiFs_unit);
	});
});

var _NomiFs_list = F2(function(path, fs)
{
	return _NomiFs_run(path, fs, _NomiFs_reads, function(store, names) {
		return store.list(fs.root, names).then(function(entries) {
			return __List_fromArray(entries.sort());
		});
	});
});

// The root itself is not removed: it is what the capability is.
var _NomiFs_remove = F2(function(path, fs)
{
	return _NomiFs_run(path, fs, _NomiFs_writes, function(store, names) {
		return names.length === 0
			? Promise.reject(_NomiFs_code('EPERM'))
			: store.remove(fs.root, names).then(_NomiFs_unit);
	});
});

var _NomiFs_makeDirectory = F2(function(path, fs)
{
	return _NomiFs_run(path, fs, _NomiFs_writes, function(store, names) {
		return store.makeDirectory(fs.root, names).then(_NomiFs_unit);
	});
});

// Whatever goes wrong, including a path out of the root, is `False`.
var _NomiFs_exists = F2(function(path, fs)
{
	var names = fs.outside === null ? _NomiFs_names(path) : null;
	if (names === null) return _Runtime.succeed(false);
	var exists = _Runtime.fromPromise(function() { return fs.store.exists(fs.root, names); });
	return _Runtime.onError(exists, function() { return _Runtime.succeed(false); });
});

var _NomiFs_reads = false;
var _NomiFs_writes = true;

function _NomiFs_unit()
{
	return __Utils_Tuple0;
}

// The operation `f` on `fs` at `path`, given the store, the names `path`
// leads to and an abort signal, as a task failing with an `Fs.Error` that
// names `path`.
function _NomiFs_run(path, fs, writes, f)
{
	if (fs.outside !== null) return _Runtime.fail(__Fs_OutsideRoot(fs.outside));
	var names = _NomiFs_names(path);
	if (names === null) return _Runtime.fail(__Fs_OutsideRoot(path));
	if (writes && !fs.writable) return _Runtime.fail(__Fs_PermissionDenied(path));
	var run = _Runtime.fromPromise(function(signal) { return f(fs.store, names, signal); });
	return _Runtime.onError(run, function(error) {
		return _Runtime.fail(_NomiFs_error(path, error));
	});
}

// What a store's error means for `path`. Node's errors and the memory
// store's carry the same codes; `OUTSIDE` is a symbolic link's way out.
function _NomiFs_error(path, error)
{
	switch (error.code)
	{
		case 'ENOENT':
			return __Fs_NotFound(path);
		case 'EACCES':
		case 'EPERM':
		case 'EROFS':
			return __Fs_PermissionDenied(path);
		case 'EEXIST':
			return __Fs_AlreadyExists(path);
		case 'EISDIR':
			return __Fs_IsADirectory(path);
		case 'OUTSIDE':
			return __Fs_OutsideRoot(path);
		case 'ENOTDIR':
			return __Fs_Other(path + ': not a directory');
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


// ON DISK

var _NomiFs_fs = process.getBuiltinModule('node:fs');
var _NomiFs_promises = _NomiFs_fs.promises;
var _NomiFs_path = process.getBuiltinModule('node:path');

// Opening a symbolic link with it fails, where the platform has it.
var _NomiFs_noFollow = _NomiFs_fs.constants.O_NOFOLLOW || 0;

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
	var real = null;
	var resolve = function(root, names, follow)
	{
		real = real || _NomiFs_promises.realpath(base);
		return _NomiFs_resolve(real, root, names, follow);
	};
	var flags = _NomiFs_fs.constants;
	return {
		read: function(root, names, signal) {
			return resolve(root, names, true).then(function(path) {
				return _NomiFs_open(_NomiFs_promises.readFile(path, { flag: flags.O_RDONLY | _NomiFs_noFollow, signal: signal }));
			});
		},
		write: function(root, names, bytes, append, signal) {
			var flag = flags.O_WRONLY | flags.O_CREAT | (append ? flags.O_APPEND : flags.O_TRUNC) | _NomiFs_noFollow;
			return resolve(root, names, true).then(function(path) {
				return _NomiFs_open(_NomiFs_promises.writeFile(path, bytes, { flag: flag, signal: signal }));
			});
		},
		list: function(root, names) {
			return resolve(root, names, true).then(function(path) {
				return _NomiFs_promises.readdir(path);
			});
		},
		// The last name is not followed, so a symbolic link is removed
		// rather than what it leads to.
		remove: function(root, names) {
			return resolve(root, names, false).then(function(path) {
				return _NomiFs_promises.rm(path, { recursive: true });
			});
		},
		makeDirectory: function(root, names) {
			return resolve(root, names, true).then(function(path) {
				return _NomiFs_promises.mkdir(path, { recursive: true });
			});
		},
		exists: function(root, names) {
			return resolve(root, names, true).then(function(path) {
				return _NomiFs_promises.stat(path);
			}).then(function() { return true; });
		}
	};
}

// The path `names` leads to from `root`, under `base`, whose real path
// `baseReal` is: the real path of the longest part of it that exists,
// followed by the names that don't, or by the last name when `follow` is
// false. That part must be inside the root, and each subdirectory the
// root is made of inside the one before it, from `base`.
function _NomiFs_resolve(baseReal, root, names, follow)
{
	var path = _NomiFs_path;
	var within = function(dir, below)
	{
		return _NomiFs_promises.realpath(path.join.apply(null, [dir].concat(below))).then(function(real) {
			if (!_NomiFs_inside(dir, real)) throw _NomiFs_code('OUTSIDE');
			return real;
		});
	};
	var rootReal = root.reduce(function(dir, below) {
		return dir.then(function(dir) { return within(dir, below); });
	}, baseReal);
	var existing = function(rootReal, n)
	{
		if (n === 0) return Promise.resolve(rootReal);
		return within(rootReal, names.slice(0, n)).catch(function(error) {
			if (error.code !== 'ENOENT' && error.code !== 'ENOTDIR') throw error;
			return existing(rootReal, n - 1).then(function(parent) { return path.join(parent, names[n - 1]); });
		});
	};
	return rootReal.then(function(rootReal) {
		return follow
			? existing(rootReal, names.length)
			: existing(rootReal, names.length - 1).then(function(parent) { return path.join(parent, names[names.length - 1]); });
	});
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

// What a directory is in the memory store; a file is its bytes.
var _NomiFs_directory = { directory: true };

function _NomiFs_key(names)
{
	return names.join('/');
}

// Files and directories by their names joined with `/`, the root's being
// empty. Each operation fails with the code the disk would give.
function _NomiFs_inMemory()
{
	var entries = new Map([['', _NomiFs_directory]]);

	// The entry at `names`, or undefined; each name before the last must be
	// a directory.
	var lookup = function(names)
	{
		for (var i = 0; i < names.length; i++)
		{
			var entry = entries.get(_NomiFs_key(names.slice(0, i)));
			if (entry === undefined) throw _NomiFs_code('ENOENT');
			if (entry !== _NomiFs_directory) throw _NomiFs_code('ENOTDIR');
		}
		return entries.get(_NomiFs_key(names));
	};
	var at = function(root, names, f)
	{
		return new Promise(function(resolve) { resolve(f([].concat.apply([], root).concat(names))); });
	};
	return {
		entries: entries,
		read: function(root, names) {
			return at(root, names, function(names) {
				var entry = lookup(names);
				if (entry === undefined) throw _NomiFs_code('ENOENT');
				if (entry === _NomiFs_directory) throw _NomiFs_code('EISDIR');
				return entry.slice();
			});
		},
		write: function(root, names, bytes, append) {
			return at(root, names, function(names) {
				var entry = lookup(names);
				if (entry === _NomiFs_directory) throw _NomiFs_code('EISDIR');
				var file = new Uint8Array(append && entry ? entry.length + bytes.length : bytes.length);
				append && entry ? (file.set(entry), file.set(bytes, entry.length)) : file.set(bytes);
				entries.set(_NomiFs_key(names), file);
			});
		},
		list: function(root, names) {
			return at(root, names, function(names) {
				var entry = lookup(names);
				if (entry === undefined) throw _NomiFs_code('ENOENT');
				if (entry !== _NomiFs_directory) throw _NomiFs_code('ENOTDIR');
				var prefix = names.length === 0 ? '' : _NomiFs_key(names) + '/';
				var children = [];
				entries.forEach(function(_, key) {
					if (key !== '' && key.startsWith(prefix) && key.indexOf('/', prefix.length) < 0) children.push(key.slice(prefix.length));
				});
				return children;
			});
		},
		remove: function(root, names) {
			return at(root, names, function(names) {
				if (lookup(names) === undefined) throw _NomiFs_code('ENOENT');
				var key = _NomiFs_key(names);
				entries.forEach(function(_, other) {
					if (other === key || other.startsWith(key + '/')) entries.delete(other);
				});
			});
		},
		makeDirectory: function(root, names) {
			return at(root, names, function(names) {
				for (var i = 1; i <= names.length; i++)
				{
					var key = _NomiFs_key(names.slice(0, i));
					var entry = entries.get(key);
					if (entry === undefined) entries.set(key, _NomiFs_directory);
					else if (entry !== _NomiFs_directory) throw _NomiFs_code(i === names.length ? 'EEXIST' : 'ENOTDIR');
				}
			});
		},
		exists: function(root, names) {
			return at(root, names, function(names) {
				return lookup(names) !== undefined;
			});
		}
	};
}
