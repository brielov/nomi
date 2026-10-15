/*

import Elm.Kernel.List exposing (fromArray)
import Elm.Kernel.Platform exposing (export)
import Elm.Kernel.Scheduler exposing (runtime)
import Elm.Kernel.Utils exposing (Tuple2)

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

var _Nomi_ensure = F2(function(finalizer, task)
{
	return _Runtime.ensuring(task, finalizer);
});
