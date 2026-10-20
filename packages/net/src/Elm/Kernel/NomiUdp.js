/*

import Elm.Kernel.NomiNet exposing (allows, key)
import Elm.Kernel.NomiStream exposing (bytes, view)
import Elm.Kernel.Utils exposing (Tuple0)
import Nomi.Udp as Udp exposing (NotAllowed, AddressInUse, PermissionDenied, Closed, Other)

*/


// SOCKETS

// The most datagrams a socket holds that no `receive` has taken.
var _NomiUdp_most = 1000;

var _NomiUdp_net = process.getBuiltinModule('node:net');

// A socket: the network it was bound through, the port it is bound to,
// the datagrams received that no `receive` has taken, oldest first, the
// fibers waiting for one, whether a wait is for the host, whether it is
// closed, `transmit(bytes, host, port)`, which sends a datagram, and
// `shut()`, which closes it.
function _NomiUdp_socket(net, port, host)
{
	return { net: net, port: port, held: [], waiting: [], host: host, closed: false, transmit: null, shut: null };
}

function _NomiUdp_address(host, port)
{
	return (host.indexOf(':') >= 0 ? '[' + host + ']' : host) + ':' + port;
}

// Binding a port of its own is listening; port 0 is a client's.
var _NomiUdp_bind = F3(function(host, port, net)
{
	var address = _NomiUdp_address(host, port);
	if (!(port >= 0 && port <= 65535 && Math.floor(port) === port))
	{
		return _Runtime.fail(__Udp_Other('port ' + port + ' is out of range: a port is 0 to 65535'));
	}
	if (port !== 0 && !net.listen) return _Runtime.fail(__Udp_PermissionDenied(address));
	var bound = net.loopback !== null ? _NomiUdp_bindLoopback(net, host, port, address) : _NomiUdp_bindBun(net, host, port, address);
	return _Runtime.acquireRelease(bound, function(socket) {
		return _Runtime.sync(function() {
			_NomiUdp_close(socket);
			return __Utils_Tuple0;
		});
	});
});

function _NomiUdp_close(socket)
{
	if (socket.closed) return;
	socket.closed = true;
	socket.shut();
	_NomiUdp_tell(socket);
}

function _NomiUdp_localPort(socket)
{
	return socket.port;
}

// Keeps a datagram for the next `receive`, or drops it past the most held.
function _NomiUdp_received(socket, bytes, host, port)
{
	if (socket.closed || socket.held.length >= _NomiUdp_most) return;
	socket.held.push({ __$data: bytes, __$host: host, __$port_: port });
	_NomiUdp_tell(socket);
}

function _NomiUdp_tell(socket)
{
	while (socket.waiting.length > 0 && (socket.held.length > 0 || socket.closed))
	{
		var resume = socket.waiting.shift();
		resume(socket.held.length > 0 ? _Runtime.succeed(socket.held.shift()) : _Runtime.fail(__Udp_Closed));
	}
}

function _NomiUdp_receive(socket)
{
	var wait = socket.host ? _Runtime.waitHost : _Runtime.wait;
	return wait(function(resume) {
		socket.waiting.push(resume);
		_NomiUdp_tell(socket);
		return function() {
			var at = socket.waiting.indexOf(resume);
			at < 0 || socket.waiting.splice(at, 1);
		};
	});
}

// Sends through the network the socket was bound through, which must allow
// the host.
var _NomiUdp_send = F4(function(host, port, bytes, socket)
{
	if (!__NomiNet_allows(socket.net, host)) return _Runtime.fail(__Udp_NotAllowed(host));
	return _Runtime.andThen(_Runtime.succeed(__Utils_Tuple0), function() {
		return socket.closed ? _Runtime.fail(__Udp_Closed) : socket.transmit(bytes, host, port);
	});
});


// ON BUN'S SOCKETS

// A socket Bun bound. A datagram goes to an address, so a name is looked
// up first; one the socket's buffer can't take yet goes once it drains.
function _NomiUdp_bindBun(net, host, port, address)
{
	return _Runtime.async(function(resume) {
		var gone = false;
		var socket = _NomiUdp_socket(net, 0, true);
		var drained = [];
		Bun.udpSocket({
			hostname: host,
			port: port,
			socket: {
				data: function(bun, data, fromPort, fromAddress) {
					_NomiUdp_received(socket, __NomiStream_bytes(new Uint8Array(data)), fromAddress, fromPort);
				},
				drain: function() {
					drained.splice(0).forEach(function(go) { go(); });
				}
			}
		}).then(function(bun) {
			socket.port = bun.port;
			socket.shut = function() { bun.close(); };
			socket.transmit = function(bytes, to, toPort) {
				var addressed = _NomiUdp_net.isIP(to) ? Promise.resolve(to) : Bun.dns.lookup(to, { family: 4 }).then(function(found) { return found[0].address; });
				return _Runtime.onError(_Runtime.fromPromise(function() {
					return addressed.then(function(ip) {
						return new Promise(function(sent) {
							var go = function() {
								if (socket.closed || bun.send(__NomiStream_view(bytes), toPort, ip)) return sent(__Utils_Tuple0);
								drained.push(go);
							};
							go();
						});
					});
				}), function(error) {
					return _Runtime.fail(__Udp_Other(_NomiUdp_address(to, toPort) + ': ' + (error && error.message || error)));
				});
			};
			gone ? _NomiUdp_close(socket) : resume(_Runtime.succeed(socket));
		}, function(error) {
			gone || resume(_Runtime.fail(error && error.code === 'EADDRINUSE' ? __Udp_AddressInUse(address)
				: error && (error.code === 'EACCES' || error.code === 'EPERM') ? __Udp_PermissionDenied(address)
				: __Udp_Other(address + ': ' + (error && error.message || error))));
		});
		return function() { gone = true; };
	});
}


// ON A LOOPBACK

// A socket on the loopback, under its host and port, one the loopback
// picks for port 0: a datagram to one that's there arrives at once, and
// one to none is dropped.
function _NomiUdp_bindLoopback(net, host, port, address)
{
	return _Runtime.andThen(_Runtime.succeed(__Utils_Tuple0), function() {
		var loopback = net.loopback;
		var taken = port === 0 ? loopback.next++ : port;
		var key = __NomiNet_key(host, taken);
		if (loopback.datagrams.has(key)) return _Runtime.fail(__Udp_AddressInUse(address));
		var socket = _NomiUdp_socket(net, taken, false);
		loopback.datagrams.set(key, socket);
		socket.shut = function() { loopback.datagrams.delete(key); };
		socket.transmit = function(bytes, to, toPort) {
			return _Runtime.sync(function() {
				var peer = loopback.datagrams.get(__NomiNet_key(to, toPort));
				peer && _NomiUdp_received(peer, bytes, '127.0.0.1', socket.port);
				return __Utils_Tuple0;
			});
		};
		return _Runtime.succeed(socket);
	});
}
