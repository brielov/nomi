/*

import Elm.Kernel.Utils exposing (Tuple0)

*/


// elm/core's `sleep` on the fiber runtime's clock, where a delay that
// isn't positive yields to the event loop rather than waiting for a
// timer.

function _Process_sleep(time)
{
	return _Runtime.map(_Runtime.sleep(time), function() {
		return __Utils_Tuple0;
	});
}
