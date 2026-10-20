/*

import Elm.Kernel.List exposing (fromArray)
import Elm.Kernel.NomiNet exposing (allows)
import Nomi.Dns as Dns exposing (NotAllowed, NotFound, Other)

*/


// NAMES

var _NomiDns_net = process.getBuiltinModule('node:net');

// The addresses `name` goes by, as Bun's resolver gives them; on a
// loopback, this machine's names alone.
var _NomiDns_lookup = F2(function(name, net)
{
	if (!__NomiNet_allows(net, name)) return _Runtime.fail(__Dns_NotAllowed(name));
	if (net.loopback !== null)
	{
		if (_NomiDns_net.isIP(name)) return _Runtime.succeed(__List_fromArray([name]));
		return name.toLowerCase() === 'localhost'
			? _Runtime.succeed(__List_fromArray(['127.0.0.1']))
			: _Runtime.fail(__Dns_NotFound(name));
	}
	var found = _Runtime.fromPromise(function() { return Bun.dns.lookup(name); });
	return _Runtime.onError(_Runtime.map(found, function(records) {
		return __List_fromArray(records.map(function(record) { return record.address; }));
	}), function(error) {
		return _Runtime.fail(error && (error.code === 'ENOTFOUND' || error.code === 'DNS_ENOTFOUND' || error.code === 'EAI_NONAME')
			? __Dns_NotFound(name)
			: __Dns_Other(name + ': ' + (error && error.message || error)));
	});
});
