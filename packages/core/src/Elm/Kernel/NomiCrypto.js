/*

import Elm.Kernel.NomiStream exposing (bytes, view)
import Elm.Kernel.Utils exposing (Tuple0)

*/


// HASHES

// Bun's name for each of `Nomi.Crypto.Algorithm`'s, which the module gives.
function _NomiCrypto_hasher(name, key)
{
	return key === null ? new Bun.CryptoHasher(name) : new Bun.CryptoHasher(name, __NomiStream_view(key));
}

var _NomiCrypto_hash = F2(function(name, data)
{
	return __NomiStream_bytes(_NomiCrypto_hasher(name, null).update(__NomiStream_view(data)).digest());
});

var _NomiCrypto_hmac = F3(function(name, key, data)
{
	return __NomiStream_bytes(_NomiCrypto_hasher(name, key).update(__NomiStream_view(data)).digest());
});

var _NomiCrypto_crypto = process.getBuiltinModule('node:crypto');

// Takes as long whatever bytes differ; bytes of different lengths differ,
// which takes no longer.
var _NomiCrypto_equalConstantTime = F2(function(a, b)
{
	return a.byteLength === b.byteLength && _NomiCrypto_crypto.timingSafeEqual(__NomiStream_view(a), __NomiStream_view(b));
});

// A hash being computed, a chunk at a time.
function _NomiCrypto_start(name)
{
	return _Runtime.sync(function() {
		return _NomiCrypto_hasher(name, null);
	});
}

var _NomiCrypto_update = F2(function(data, hasher)
{
	return _Runtime.sync(function() {
		hasher.update(__NomiStream_view(data));
		return __Utils_Tuple0;
	});
});

function _NomiCrypto_digest(hasher)
{
	return _Runtime.sync(function() {
		return __NomiStream_bytes(hasher.digest());
	});
}


// RANDOMNESS

// Fills `array` with random bytes: the system's, or under `nomi test` the
// test runtime's own generator, seeded by the test, so each run of a test
// gets the same values.
function _NomiCrypto_fill(runtime, array)
{
	runtime.random ? runtime.random(array) : crypto.getRandomValues(array);
	return array;
}

function _NomiCrypto_random(size)
{
	return _Runtime.withRuntime(function(runtime) {
		return _Runtime.sync(function() {
			return __NomiStream_bytes(_NomiCrypto_fill(runtime, new Uint8Array(Math.max(0, size))));
		});
	});
}

// An integer from `low` to `high`, each as likely, by rejecting the values
// that would favour some.
var _NomiCrypto_int = F2(function(low, high)
{
	return _Runtime.withRuntime(function(runtime) {
		return _Runtime.sync(function() {
			var lo = Math.min(low, high);
			var span = Math.max(low, high) - lo + 1;
			if (span <= 1) return lo;
			var limit = Math.floor(4294967296 / span) * span;
			var word = new Uint32Array(1);
			do { _NomiCrypto_fill(runtime, new Uint8Array(word.buffer)); } while (word[0] >= limit);
			return lo + word[0] % span;
		});
	});
});

function _NomiCrypto_hex(array)
{
	var hex = Array.from(array, function(byte) { return byte.toString(16).padStart(2, '0'); }).join('');
	return hex.slice(0, 8) + '-' + hex.slice(8, 12) + '-' + hex.slice(12, 16) + '-' + hex.slice(16, 20) + '-' + hex.slice(20);
}

// A version 4 UUID: 122 random bits.
var _NomiCrypto_uuid = _Runtime.withRuntime(function(runtime) {
	return _Runtime.sync(function() {
		var array = _NomiCrypto_fill(runtime, new Uint8Array(16));
		array[6] = array[6] & 0x0f | 0x40;
		array[8] = array[8] & 0x3f | 0x80;
		return _NomiCrypto_hex(array);
	});
});

// A version 7 UUID: the clock's milliseconds, which sort it by when it was
// made, then 74 random bits.
var _NomiCrypto_uuidV7 = _Runtime.withRuntime(function(runtime) {
	return _Runtime.andThen(_Runtime.now, function(now) {
		return _Runtime.sync(function() {
			var array = _NomiCrypto_fill(runtime, new Uint8Array(16));
			var ms = Math.floor(now);
			for (var i = 5; i >= 0; i--)
			{
				array[i] = ms % 256;
				ms = Math.floor(ms / 256);
			}
			array[6] = array[6] & 0x0f | 0x70;
			array[8] = array[8] & 0x3f | 0x80;
			return _NomiCrypto_hex(array);
		});
	});
});


// PASSWORDS

function _NomiCrypto_hashPassword(password)
{
	return _Runtime.fromPromise(function() {
		return Bun.password.hash(password, { algorithm: 'argon2id' });
	});
}

// A hash Bun can't read is a password that doesn't match it.
var _NomiCrypto_verifyPassword = F2(function(password, hash)
{
	return _Runtime.fromPromise(function() {
		return Bun.password.verify(password, hash).catch(function() { return false; });
	});
});
