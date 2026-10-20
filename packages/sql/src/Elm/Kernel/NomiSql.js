/*

import Elm.Kernel.List exposing (fromArray, toArray)
import Elm.Kernel.NomiFs exposing (onDisk)
import Elm.Kernel.NomiStream exposing (bytes, view)
import Elm.Kernel.Utils exposing (Tuple0, Tuple2)
import Nomi.Sql as Sql exposing (FileError, NotOnDisk, Violated, Failed, BadParameter, Closed, Null, Int, Int64, Float, Text, Blob, Bool, Unique, ForeignKey, NotNull, Check)

*/


// DATABASES

// A database: Bun's handle; whether it is closed; the transaction that
// holds it, if one does, whose statements alone run meanwhile; and the
// fibers waiting for it to let go. A transaction is a `Db` too, of the
// database it is on, `root`, open until it ends: one within another is a
// savepoint, of the depth it is at.
function _NomiSql_database(sql)
{
	var db = { sql: sql, closed: false, holder: null, waiting: [], depth: 0, ended: false };
	db.root = db;
	return db;
}

// Opens SQLite on `filename` with integers as `bigint`s, enforcing
// foreign keys, owned by the fiber that opens it.
function _NomiSql_sqlite(options)
{
	var opened = _Runtime.fromPromise(function() {
		var sql = new Bun.SQL(Object.assign({ adapter: 'sqlite', safeIntegers: true }, options));
		// Opening is lazy: these find a file that isn't a database.
		return sql.unsafe('PRAGMA foreign_keys = ON')
			.then(function() { return sql.unsafe('PRAGMA schema_version'); })
			.then(function() { return _NomiSql_database(sql); }, function(error) {
				sql.close();
				throw error;
			});
	});
	return _Runtime.acquireRelease(_Runtime.onError(opened, function(error) {
		return _Runtime.fail(_NomiSql_error(error));
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

// Closes the database: the fibers waiting for a transaction to let go of
// it are woken, to find it closed.
function _NomiSql_shut(db)
{
	return _Runtime.fromPromise(function() {
		if (db.closed) return __Utils_Tuple0;
		db.closed = true;
		_NomiSql_letGo(db);
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


// STATEMENTS

// Runs `f` with Bun's handle once `db` may: a transaction's statements at
// once, while it is open; the database's once no transaction holds it.
function _NomiSql_turn(db, f)
{
	var root = db.root;
	// Checked as the statement runs, not as its task is made.
	var run = _Runtime.andThen(_Runtime.succeed(__Utils_Tuple0), function() {
		if (root.closed || db.ended) return _Runtime.fail(__Sql_Closed);
		return _Runtime.onError(_Runtime.fromPromise(function() { return f(root.sql); }), function(error) {
			return _Runtime.fail(_NomiSql_error(error));
		});
	});
	return db === root ? _Runtime.andThen(_NomiSql_free(root), function() { return run; }) : run;
}

// Succeeds once no transaction holds the database.
function _NomiSql_free(root)
{
	if (root.holder === null) return _Runtime.succeed(__Utils_Tuple0);
	return _Runtime.andThen(_Runtime.wait(function(resume) {
		root.waiting.push(resume);
		return function() {
			var at = root.waiting.indexOf(resume);
			at < 0 || root.waiting.splice(at, 1);
		};
	}), function() { return _NomiSql_free(root); });
}

// Lets go of the database, waking the fibers waiting for it.
function _NomiSql_letGo(root)
{
	root.holder = null;
	root.waiting.splice(0).forEach(function(resume) {
		resume(_Runtime.succeed(__Utils_Tuple0));
	});
}

// Runs the statement on `db` once it may, with the parameters checked first.
function _NomiSql_run(text, parameters, db)
{
	var values = [];
	var given = __List_toArray(parameters);
	for (var i = 0; i < given.length; i++)
	{
		if (given[i].refused !== undefined) return _Runtime.fail(A2(__Sql_BadParameter, i + 1, given[i].refused));
		values.push(given[i].value);
	}
	return _NomiSql_turn(db, function(sql) { return sql.unsafe(text, values); });
}

var _NomiSql_query = F3(function(text, parameters, db)
{
	return _Runtime.map(_NomiSql_run(text, parameters, db), function(rows) {
		return __List_fromArray(Array.from(rows, function(row) {
			return __List_fromArray(Object.keys(row).map(function(name) {
				return __Utils_Tuple2(name, _NomiSql_value(row[name]));
			}));
		}));
	});
});

var _NomiSql_execute = F3(function(text, parameters, db)
{
	return _Runtime.map(_NomiSql_run(text, parameters, db), function(result) {
		return { __$changes: Number(result.count || 0) };
	});
});


// TRANSACTIONS

// Runs `f` in a transaction on `db`: BEGIN, or a savepoint within another,
// then COMMIT once `f` succeeds, and a rollback however else it ends. The
// rollback is a finalizer of the scope `f` runs in, so an interruption
// takes it too; it skips a transaction already committed.
var _NomiSql_transaction = F3(function(toError, f, db)
{
	var root = db.root;
	var tx = { root: root, depth: db.depth + 1, ended: false, committed: false };
	var name = 'nomi_' + tx.depth;
	var statement = function(text) {
		return _Runtime.onError(_NomiSql_turn(tx, function(sql) { return sql.unsafe(text); }), function(error) {
			return _Runtime.fail(toError(error));
		});
	};
	var begin = _Runtime.andThen(db === root ? _NomiSql_free(root) : _Runtime.succeed(__Utils_Tuple0), function() {
		if (root.closed || db.ended) return _Runtime.fail(toError(__Sql_Closed));
		if (db === root) root.holder = tx;
		return _Runtime.onError(statement(db === root ? 'BEGIN' : 'SAVEPOINT ' + name), function(error) {
			db === root && _NomiSql_letGo(root);
			return _Runtime.fail(error);
		});
	});
	var end = function()
	{
		var rollback = tx.committed || root.closed
			? _Runtime.succeed(__Utils_Tuple0)
			: _Runtime.fromPromise(function() {
				var text = db === root ? 'ROLLBACK' : 'ROLLBACK TO ' + name + '; RELEASE ' + name;
				return root.sql.unsafe(text).simple().then(_NomiSql_unit, _NomiSql_unit);
			});
		return _Runtime.map(rollback, function() {
			tx.ended = true;
			db === root && _NomiSql_letGo(root);
			return __Utils_Tuple0;
		});
	};
	var commit = function(value)
	{
		return _Runtime.uninterruptible(_Runtime.map(statement(db === root ? 'COMMIT' : 'RELEASE ' + name), function() {
			tx.committed = true;
			return value;
		}));
	};
	return _Runtime.scoped(_Runtime.andThen(_Runtime.acquireRelease(begin, end), function() {
		return _Runtime.andThen(f(tx), commit);
	}));
});


// VALUES

// The parameters, as the Elm side makes them: what Bun is given, or why
// one is refused.
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

// SQLite stores NaN as NULL, so it is refused rather than lost.
function _NomiSql_float(f)
{
	return Number.isNaN(f) ? { refused: 'NaN, which SQLite stores as NULL' } : { value: f };
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

// A column's value as Elm's `Value`.
function _NomiSql_value(cell)
{
	if (cell === null || cell === undefined) return __Sql_Null;
	switch (typeof cell)
	{
		case 'bigint':
			return cell >= -_NomiSql_safe && cell <= _NomiSql_safe ? __Sql_Int(Number(cell)) : __Sql_Int64(cell.toString());
		case 'number':
			return __Sql_Float(cell);
		case 'string':
			return __Sql_Text(cell);
		case 'boolean':
			return __Sql_Bool(cell);
	}
	if (cell instanceof Uint8Array) return __Sql_Blob(__NomiStream_bytes(cell));
	return __Sql_Text(String(cell));
}


// ERRORS

// A failure of Bun's as Elm's `Error`: a broken constraint by its kind,
// as SQLite's code says it, and any other by its code.
function _NomiSql_error(error)
{
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
		default:
			return __Sql_Failed({ __$code: error.code || error.name, __$message: error.message });
	}
}
