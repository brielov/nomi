/*

import Elm.Kernel.List exposing (toArray)

*/


// NETWORKS

// A network capability: the hosts it may reach, by lower-case name, or
// null for any; whether it may listen; and for a loopback, its HTTP
// servers by host and port, each a function answering a request, its TCP
// listeners by host and port or Unix path, each taking connections, and
// the port the next one picking a port takes.
var _NomiNet_real = { hosts: null, listen: true, loopback: null };

var _NomiNet_onlyHosts = F2(function(hosts, net)
{
	var wanted = __List_toArray(hosts).map(function(host) { return host.toLowerCase(); });
	return {
		hosts: net.hosts === null ? wanted : wanted.filter(function(host) { return net.hosts.indexOf(host) >= 0; }),
		listen: net.listen,
		loopback: net.loopback
	};
});

function _NomiNet_noListen(net)
{
	return { hosts: net.hosts, listen: false, loopback: net.loopback };
}

var _NomiNet_loopback = _Runtime.sync(function() {
	return { hosts: null, listen: true, loopback: { servers: new Map(), listeners: new Map(), next: 49152 } };
});

// Whether `net` may reach `host`.
function _NomiNet_allows(net, host)
{
	return net.hosts === null || net.hosts.indexOf(host.toLowerCase()) >= 0;
}

// A loopback's servers are known by host and port, every name of this
// machine being one host.
function _NomiNet_key(host, port)
{
	var name = host.toLowerCase();
	return (_NomiNet_local[name] ? 'localhost' : name) + ':' + port;
}

var _NomiNet_local = { 'localhost': true, '127.0.0.1': true, '::1': true, '[::1]': true, '0.0.0.0': true, '::': true };
