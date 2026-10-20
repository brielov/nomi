/*

import Time exposing (customZone, Name, Offset)
import Elm.Kernel.List exposing (Nil)
import Elm.Kernel.Scheduler exposing (binding, succeed)

*/


// elm/time's clock on the fiber runtime's: `now` reads it, and an
// interval is a loop that sleeps on it and starts `task` on a daemon each
// time it wakes, at least a millisecond apart, as upstream's
// `setInterval` is on Bun. The time zone functions are upstream's.

function _Time_now(millisToPosix)
{
	return _Runtime.map(_Runtime.now, millisToPosix);
}

var _Time_setInterval = F2(function(interval, task)
{
	var tick = _Runtime.andThen(_Runtime.sleep(interval > 1 ? interval : 1), function() {
		return _Runtime.andThen(_Runtime.forkDaemon(task), function() {
			return tick;
		});
	});
	return tick;
});

function _Time_here()
{
	return __Scheduler_binding(function(callback)
	{
		callback(__Scheduler_succeed(
			A2(__Time_customZone, -(new Date().getTimezoneOffset()), __List_Nil)
		));
	});
}


function _Time_getZoneName()
{
	return __Scheduler_binding(function(callback)
	{
		try
		{
			var name = __Time_Name(Intl.DateTimeFormat().resolvedOptions().timeZone);
		}
		catch (e)
		{
			var name = __Time_Offset(new Date().getTimezoneOffset());
		}
		callback(__Scheduler_succeed(name));
	});
}
