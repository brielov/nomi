/*

import Elm.Kernel.NomiFs exposing (memory)
import Elm.Kernel.NomiStdio exposing (script, capture, captured)

*/


function _NomiFake_fs(files)
{
	return _Runtime.sync(function() { return __NomiFs_memory(files); });
}

function _NomiFake_reader(text)
{
	return _Runtime.sync(function() { return __NomiStdio_script(text); });
}

var _NomiFake_writer = _Runtime.sync(function() { return __NomiStdio_capture(null); });

function _NomiFake_captured(writer)
{
	return _Runtime.sync(function() { return __NomiStdio_captured(writer); });
}
