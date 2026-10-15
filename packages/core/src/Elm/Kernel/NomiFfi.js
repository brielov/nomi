/*

import Elm.Kernel.Json exposing (wrap, unwrap)

*/


// CALLS

// The task calls the FFI module's export `name` with the argument and an
// AbortSignal that interruption aborts, and succeeds with what its promise
// fulfils with. The module is the entry's scope's `ffi`, which it imports.
// A missing export, a throw and a rejection each fail with the message.
var _NomiFfi_call = F2(function(name, arg)
{
	var call = _Runtime.fromPromise(function(signal) {
		return new Promise(function(resolve) {
			var f = scope.ffi[name];
			if (typeof f !== 'function')
			{
				throw new Error('the FFI module exports no function `' + name + '`');
			}
			resolve(f(__Json_unwrap(arg), signal));
		});
	});
	return _Runtime.onError(_Runtime.map(call, __Json_wrap), function(error) {
		return _Runtime.fail(error instanceof Error ? error.message : String(error));
	});
});
