/*

import Elm.Kernel.List exposing (Nil, fromArray, toArray)
import Elm.Kernel.NomiFs exposing (onDisk)
import Elm.Kernel.NomiNet exposing (allows)
import Elm.Kernel.NomiStream exposing (bytes, view)
import Elm.Kernel.Utils exposing (Tuple0, Tuple2)
import Nomi.Sql as Sql exposing (FileError, NotOnDisk, NotAllowed, Unreachable, Violated, Failed, BadParameter, Closed, Null, Int, Int64, Float, Text, Blob, Bool, Unique, ForeignKey, NotNull, Check)

*/


// DATABASES

// A database: its kind, `sqlite`, `postgres` or `mysql`; Bun's handle;
// the host it is on, for errors; whether it is closed; and its pool: the
// connections idle, how many there are, idle or not, the most there may
// be, and the fibers waiting for one, each with what to start on it. A
// connection is a handle statements run on, and whether it broke:
// SQLite's is the database's own handle, its pool's only one; Postgres's
// and MySQL's one Bun reserved, kept until it breaks or the database
// closes. A transaction is a `Db` too, of the database it is on, `root`,
// on the connection it holds, `depth` transactions deep, and `ended` once
// it has.
function _NomiSql_database(kind, sql, host, most, first)
{
	var db = { kind: kind, sql: sql, host: host, closed: false, idle: [first], count: 1, most: most, waiting: [], connection: null, depth: 0, ended: false };
	db.root = db;
	return db;
}

function _NomiSql_connection(sql)
{
	return { sql: sql, broken: false };
}

// SQLite on `options.filename`, integers as `bigint`s, foreign keys
// enforced, owned by the fiber that opens it.
function _NomiSql_sqlite(options)
{
	var opened = _Runtime.fromPromise(function() {
		var sql = new Bun.SQL(Object.assign({ adapter: 'sqlite', safeIntegers: true }, options));
		// Opening is lazy: these find a file that isn't a database.
		return sql.unsafe('PRAGMA foreign_keys = ON')
			.then(function() { return sql.unsafe('PRAGMA schema_version'); })
			.then(function() {
				return _NomiSql_database('sqlite', sql, options.filename, 1, _NomiSql_connection(sql));
			}, function(error) {
				sql.close();
				throw error;
			});
	});
	return _Runtime.acquireRelease(_Runtime.onError(opened, function(error) {
		return _Runtime.fail(_NomiSql_error(null, error));
	}), _NomiSql_shut);
}

var _NomiSql_open = F2(function(path, fs)
{
	return _Runtime.andThen(_Runtime.onError(A2(__NomiFs_onDisk, path, fs), function(error) {
		return _Runtime.fail(__Sql_FileError(error));
	}), function(disk) {
		return disk === null
			? _Runtime.fail(__Sql_NotOnDisk(path))
			: _NomiSql_sqlite(disk.writable ? { filename: disk.path, create: true } : { filename: disk.path, readonly: true });
	});
});

var _NomiSql_memory = _NomiSql_sqlite({ filename: ':memory:' });

// A pool of up to `most` connections to the Postgres or MySQL server at
// `url`, through `net`, its first connection made at once. `prefer` is
// nomi's: Bun's waits out its timeout on a Postgres without TLS, so TLS is
// asked for, and the connection made without it where the server has none.
var _NomiSql_connect = F4(function(url, mode, most, net)
{
	var parsed = new URL(url);
	var kind = parsed.protocol === 'mysql:' ? 'mysql' : 'postgres';
	var host = parsed.hostname;
	// A loopback has no server but the program's own.
	if (net.loopback !== null) return _Runtime.fail(__Sql_Unreachable(host + ': connection refused'));
	if (!__NomiNet_allows(net, host)) return _Runtime.fail(__Sql_NotAllowed(host));
	var reach = function(sslmode)
	{
		parsed.searchParams.set('sslmode', sslmode);
		var sql = new Bun.SQL(parsed.href, { max: most, bigint: true });
		return sql.reserve().then(function(reserved) {
			return _NomiSql_database(kind, sql, host, most, _NomiSql_connection(reserved));
		}, function(error) {
			sql.close();
			throw error;
		});
	};
	var opened = _Runtime.fromPromise(function() {
		return mode !== 'prefer' ? reach(mode) : reach('require').catch(function(error) {
			if (error.code === 'ERR_POSTGRES_TLS_NOT_AVAILABLE') return reach('disable');
			throw error;
		});
	});
	return _Runtime.acquireRelease(_Runtime.onError(opened, function(error) {
		return _Runtime.fail(_NomiSql_error({ kind: kind, host: host, closed: false }, error));
	}), _NomiSql_shut);
});

// Closes the database: the fibers waiting for a connection are woken to
// find it closed, and each connection is let go once it is given back.
function _NomiSql_shut(db)
{
	return _Runtime.fromPromise(function() {
		if (db.closed) return __Utils_Tuple0;
		db.closed = true;
		db.waiting.splice(0).forEach(function(waiter) {
			waiter.resume(_Runtime.fail(__Sql_Closed));
		});
		db.idle.splice(0).forEach(function(connection) {
			_NomiSql_letGo(db, connection);
		});
		return Promise.resolve(db.sql.close()).then(_NomiSql_unit, _NomiSql_unit);
	});
}

function _NomiSql_close(db)
{
	return _NomiSql_shut(db.root);
}

function _NomiSql_unit()
{
	return __Utils_Tuple0;
}


// THE POOL

// Starts `start` on a free connection of `db`'s pool, succeeding with
// what it gives: at once where one is idle, once one is made where the
// pool may grow, and otherwise once another fiber gives one back. `start`
// runs as the connection is handed over, in the same turn, so no
// interruption can come between and lose it.
function _NomiSql_claim(db, start)
{
	return _Runtime.andThen(_Runtime.succeed(__Utils_Tuple0), function() {
		if (db.closed) return _Runtime.fail(__Sql_Closed);
		if (db.idle.length > 0) return _Runtime.succeed(start(db.idle.pop()));
		if (db.count < db.most) return _NomiSql_grow(db, start);
		return _Runtime.wait(function(resume) {
			var waiter = { start: start, resume: resume };
			db.waiting.push(waiter);
			return function() {
				var at = db.waiting.indexOf(waiter);
				at < 0 || db.waiting.splice(at, 1);
			};
		});
	});
}

// A new connection for the pool, `start` started on it; one made for a
// fiber interrupted meanwhile goes to the pool.
function _NomiSql_grow(db, start)
{
	db.count++;
	return _Runtime.async(function(resume) {
		var gone = false;
		db.sql.reserve().then(function(reserved) {
			var connection = _NomiSql_connection(reserved);
			gone ? _NomiSql_give(db, connection) : resume(_Runtime.succeed(start(connection)));
		}, function(error) {
			db.count--;
			_NomiSql_wake(db);
			gone || resume(_Runtime.fail(_NomiSql_error(db, error)));
		});
		return function() { gone = true; };
	});
}

// Gives the connection back: to the first fiber waiting, starting what it
// waits to start, or to the idle. One broken, or of a database since
// closed, leaves the pool, which may make another in its place.
function _NomiSql_give(db, connection)
{
	if (db.closed || connection.broken)
	{
		db.count--;
		_NomiSql_letGo(db, connection);
		_NomiSql_wake(db);
		return;
	}
	if (db.waiting.length > 0)
	{
		var waiter = db.waiting.shift();
		waiter.resume(_Runtime.succeed(waiter.start(connection)));
		return;
	}
	db.idle.push(connection);
}

// Has the first fiber waiting make a connection, where the pool may grow.
function _NomiSql_wake(db)
{
	if (db.closed || db.waiting.length === 0 || db.count >= db.most) return;
	var waiter = db.waiting.shift();
	waiter.resume(_NomiSql_grow(db, waiter.start));
}

// A connection Bun reserved goes back to Bun's pool, which closes it.
function _NomiSql_letGo(db, connection)
{
	if (connection.sql !== db.sql) connection.sql.release();
}


// STATEMENTS

// The parameters, as the Elm side makes them, checked: what Bun is given.
function _NomiSql_values(db, parameters)
{
	var values = [];
	var given = __List_toArray(parameters);
	for (var i = 0; i < given.length; i++)
	{
		var refused = given[i].refused || (db.root.kind === 'sqlite' && given[i].sqlite);
		if (refused) return { refused: A2(__Sql_BadParameter, i + 1, refused) };
		values.push(given[i].value);
	}
	return { values: values };
}

// Runs the statement on `db`, giving Bun's result: on a free connection
// of its pool, which it gives back once the statement has run, whether or
// not its fiber still waits; or on the transaction's own, while it is open.
function _NomiSql_run(db, text, parameters)
{
	var root = db.root;
	var checked = _NomiSql_values(db, parameters);
	if (checked.refused) return _Runtime.fail(checked.refused);
	var on = function(connection)
	{
		var statement = checked.values.length === 0 ? connection.sql.unsafe(text) : connection.sql.unsafe(text, checked.values);
		return statement.then(null, function(error) {
			connection.broken = connection.sql !== root.sql && _NomiSql_breaks(error);
			throw error;
		});
	};
	var started = db === root
		? _NomiSql_claim(root, function(connection) {
			var running = on(connection);
			var give = function() { _NomiSql_give(root, connection); };
			running.then(give, give);
			return running;
		})
		: _Runtime.andThen(_Runtime.succeed(__Utils_Tuple0), function() {
			return root.closed || db.ended ? _Runtime.fail(__Sql_Closed) : _Runtime.succeed(on(db.connection));
		});
	return _Runtime.andThen(started, function(running) {
		return _Runtime.onError(_Runtime.fromPromise(function() { return running; }), function(error) {
			return _Runtime.fail(_NomiSql_error(root, error));
		});
	});
}

// A script's result is each statement's: the last is the script's.
function _NomiSql_last(result)
{
	return result.length > 0 && Array.isArray(result[0]) && result.count === undefined ? result[result.length - 1] : result;
}

var _NomiSql_query = F3(function(text, parameters, db)
{
	return _Runtime.map(_NomiSql_run(db, text, parameters), function(result) {
		return __List_fromArray(Array.from(_NomiSql_last(result), function(row) {
			return __List_fromArray(Object.keys(row).map(function(name) {
				return __Utils_Tuple2(name, _NomiSql_value(row[name]));
			}));
		}));
	});
});

var _NomiSql_execute = F3(function(text, parameters, db)
{
	return _Runtime.map(_NomiSql_run(db, text, parameters), function(result) {
		var last = _NomiSql_last(result);
		// MySQL counts what changed as `affectedRows`, the others as `count`.
		return { __$changes: Number(last.affectedRows != null ? last.affectedRows : last.count || 0) };
	});
});


// TRANSACTIONS

// Runs `f` in a transaction on `db`, on a connection it holds: BEGIN, or
// a savepoint within another, then COMMIT once `f` succeeds. A finalizer
// of the scope `f` runs in rolls back however else it ends, interruption
// included, unless it committed, and gives the connection back.
var _NomiSql_transaction = F3(function(toError, f, db)
{
	var root = db.root;
	var nested = db !== root;
	var tx = { root: root, connection: null, depth: db.depth + 1, ended: false, begun: false, committed: false };
	var name = 'nomi_' + tx.depth;
	var statement = function(text) {
		return _Runtime.onError(_NomiSql_run(tx, text, __List_Nil), function(error) {
			return _Runtime.fail(toError(error));
		});
	};
	var claimed = nested
		? _Runtime.andThen(_Runtime.succeed(__Utils_Tuple0), function() {
			if (root.closed || db.ended) return _Runtime.fail(__Sql_Closed);
			tx.connection = db.connection;
			return _Runtime.succeed(__Utils_Tuple0);
		})
		: _NomiSql_claim(root, function(connection) {
			tx.connection = connection;
			return __Utils_Tuple0;
		});
	var end = function()
	{
		var connection = tx.connection;
		var rollback = connection === null || !tx.begun || tx.committed || root.closed || connection.broken
			? Promise.resolve()
			: (nested
				? connection.sql.unsafe('ROLLBACK TO SAVEPOINT ' + name).then(function() { return connection.sql.unsafe('RELEASE SAVEPOINT ' + name); })
				: connection.sql.unsafe('ROLLBACK')).then(null, function(error) {
					connection.broken = connection.sql !== root.sql && _NomiSql_breaks(error);
				});
		return _Runtime.fromPromise(function() {
			return rollback.then(function() {
				tx.ended = true;
				nested || connection === null || _NomiSql_give(root, connection);
				return __Utils_Tuple0;
			});
		});
	};
	var commit = function(value)
	{
		return _Runtime.uninterruptible(_Runtime.map(statement(nested ? 'RELEASE SAVEPOINT ' + name : 'COMMIT'), function() {
			tx.committed = true;
			return value;
		}));
	};
	return _Runtime.scoped(_Runtime.andThen(_Runtime.addFinalizer(end), function() {
		return _Runtime.andThen(_Runtime.onError(claimed, function(error) { return _Runtime.fail(toError(error)); }), function() {
			return _Runtime.andThen(statement(nested ? 'SAVEPOINT ' + name : 'BEGIN'), function() {
				tx.begun = true;
				return _Runtime.andThen(f(tx), commit);
			});
		});
	}));
});


// VALUES

// The parameters, as the Elm side makes them: what Bun is given, or why
// one is refused, or why SQLite alone refuses it.
var _NomiSql_null = { value: null };

// The integers a number holds exactly.
var _NomiSql_safe = 9007199254740991n;

function _NomiSql_int(n)
{
	return Number.isSafeInteger(n)
		? { value: BigInt(n) }
		: { refused: n + ' is past ±(2^53 − 1), where an Int is rounded: pass it with Int64' };
}

var _NomiSql_least = -(2n ** 63n);
var _NomiSql_most = 2n ** 63n - 1n;

function _NomiSql_int64(text)
{
	var n = /^-?[0-9]+$/.test(text) ? BigInt(text) : null;
	return n !== null && n >= _NomiSql_least && n <= _NomiSql_most
		? { value: n }
		: { refused: JSON.stringify(text) + ' is not a 64-bit integer in decimal' };
}

function _NomiSql_float(f)
{
	return Number.isNaN(f) ? { value: f, sqlite: 'NaN, which SQLite stores as NULL' } : { value: f };
}

function _NomiSql_text(text)
{
	return { value: text };
}

function _NomiSql_blob(bytes)
{
	return { value: __NomiStream_view(bytes) };
}

function _NomiSql_bool(b)
{
	return { value: b };
}

// A column's value as Elm's `Value`. A number that is an integer is one,
// as Postgres's `int4` and MySQL's `INT` read as numbers; a time is its
// ISO 8601 text, and JSON its text.
function _NomiSql_value(cell)
{
	if (cell === null || cell === undefined) return __Sql_Null;
	switch (typeof cell)
	{
		case 'bigint':
			return cell >= -_NomiSql_safe && cell <= _NomiSql_safe ? __Sql_Int(Number(cell)) : __Sql_Int64(cell.toString());
		case 'number':
			return Number.isSafeInteger(cell) ? __Sql_Int(cell) : __Sql_Float(cell);
		case 'string':
			return __Sql_Text(cell);
		case 'boolean':
			return __Sql_Bool(cell);
	}
	if (cell instanceof Uint8Array) return __Sql_Blob(__NomiStream_bytes(cell));
	if (cell instanceof Date) return __Sql_Text(cell.toISOString());
	return __Sql_Text(JSON.stringify(cell, function(key, value) {
		return typeof value === 'bigint' ? value.toString() : value;
	}));
}


// ERRORS

// Whether a statement's error is its connection's rather than its own:
// one the database answered carries the database's code, and one without
// it, the connection lost or reserved no more, means the connection broke.
function _NomiSql_breaks(error)
{
	return error.errno === undefined;
}

// A failure of Bun's as Elm's `Error`: a broken constraint by its kind, a
// statement the database refused by its code, and one that never reached
// it as the server's to reach. `db` is the database, or what is known of
// it while it opens; none, for SQLite.
function _NomiSql_error(db, error)
{
	if (db !== null && db.kind === 'postgres' && typeof error.errno === 'string')
	{
		var constraint = { '23505': __Sql_Unique, '23503': __Sql_ForeignKey, '23502': __Sql_NotNull, '23514': __Sql_Check }[error.errno];
		return constraint ? A2(__Sql_Violated, constraint, error.message) : __Sql_Failed({ __$code: error.errno, __$message: error.message });
	}
	if (db !== null && db.kind === 'mysql' && typeof error.errno === 'number')
	{
		switch (error.errno)
		{
			case 1062:
				return A2(__Sql_Violated, __Sql_Unique, error.message);
			case 1216:
			case 1217:
			case 1451:
			case 1452:
				return A2(__Sql_Violated, __Sql_ForeignKey, error.message);
			case 1048:
			case 1364:
				return A2(__Sql_Violated, __Sql_NotNull, error.message);
			case 3819:
				return A2(__Sql_Violated, __Sql_Check, error.message);
			default:
				return __Sql_Failed({ __$code: error.sqlState || String(error.errno), __$message: error.message });
		}
	}
	switch (error.code)
	{
		case 'SQLITE_CONSTRAINT_UNIQUE':
		case 'SQLITE_CONSTRAINT_PRIMARYKEY':
			return A2(__Sql_Violated, __Sql_Unique, error.message);
		case 'SQLITE_CONSTRAINT_FOREIGNKEY':
			return A2(__Sql_Violated, __Sql_ForeignKey, error.message);
		case 'SQLITE_CONSTRAINT_NOTNULL':
			return A2(__Sql_Violated, __Sql_NotNull, error.message);
		case 'SQLITE_CONSTRAINT_CHECK':
			return A2(__Sql_Violated, __Sql_Check, error.message);
		case 'ERR_SQLITE_CONNECTION_CLOSED':
			return __Sql_Closed;
		case 'ERR_MYSQL_PUBLIC_KEY_RETRIEVAL_NOT_ALLOWED':
			return __Sql_Unreachable(db.host + ': MySQL takes a password only over TLS, but the connection has none: connect with sslmode prefer or require');
	}
	if (db === null || db.kind === 'sqlite') return __Sql_Failed({ __$code: error.code || error.name, __$message: error.message });
	if (db.closed) return __Sql_Closed;
	return __Sql_Unreachable(db.host + ': ' + error.message);
}
