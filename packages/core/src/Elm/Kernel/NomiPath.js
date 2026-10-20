/*

*/


// GLOBS

// Each glob's compiled matcher, by its text, as a program matches many
// paths against one glob.
var _NomiPath_globs = new Map();

var _NomiPath_matches = F2(function(glob, path)
{
	var matcher = _NomiPath_globs.get(glob);
	if (matcher === undefined)
	{
		matcher = new Bun.Glob(glob);
		_NomiPath_globs.size < 1000 && _NomiPath_globs.set(glob, matcher);
	}
	return matcher.match(path);
});
