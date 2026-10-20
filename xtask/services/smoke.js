// The smoke test `cargo xtask services` runs against each service once it
// is up: `bun smoke.js <service>`, with the service's URL, credentials
// included, in NOMI_SERVICE_URL. It talks to the service through Bun's
// own clients, which nomi's drivers are built on, and exits 1 at the
// first answer that isn't the one expected.

import { createHash, createHmac } from 'node:crypto';

const smoke = { postgres, mysql, valkey, garage };

const [service] = process.argv.slice(2);
await smoke[service](process.env.NOMI_SERVICE_URL);

function same(actual, expected, what)
{
	if (JSON.stringify(actual) !== JSON.stringify(expected))
	{
		throw new Error(`${what}: expected ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}`);
	}
}


// SQL

async function postgres(url)
{
	await roundTrip(new Bun.SQL(url));
}

// MySQL 9 signs in with caching_sha2_password, whose first sign-in sends
// the password only over TLS, or encrypted with a key the server hands
// out, which Bun refuses unless asked. The server makes its own
// certificate, which no one signed.
async function mysql(url)
{
	await roundTrip(new Bun.SQL(url, { tls: { rejectUnauthorized: false } }));
}

async function roundTrip(sql)
{
	await sql`create table smoke (id int primary key, name varchar(20) not null)`;
	await sql`insert into smoke ${sql([{ id: 1, name: 'one' }, { id: 2, name: 'two' }])}`;
	const rows = await sql`select id, name from smoke order by id`;
	same(rows.map(row => [row.id, row.name]), [[1, 'one'], [2, 'two']], 'the rows read back');
	await sql`drop table smoke`;
	await sql.close();
}


// VALKEY

// Bun's client reconnects for ever unless told not to, which would hang
// the test on a service that isn't there.
async function valkey(url)
{
	const redis = new Bun.RedisClient(url, { autoReconnect: false });
	same(await redis.send('PING', []), 'PONG', 'PING');
	await redis.set('smoke', '1');
	same(await redis.incr('smoke'), 2, 'INCR');
	same(await redis.get('smoke'), '2', 'GET');
	same(await redis.del('smoke'), 1, 'DEL');
	redis.close();
}


// GARAGE

// The URL is `http://<key id>:<secret>@<host>/<bucket>?region=<region>`.
async function garage(url)
{
	const given = new URL(url);
	const s3 = {
		accessKeyId: decodeURIComponent(given.username),
		secretAccessKey: decodeURIComponent(given.password),
		bucket: given.pathname.slice(1),
		endpoint: given.origin,
		region: given.searchParams.get('region')
	};
	const client = new Bun.S3Client(s3);

	await client.write('smoke.txt', 'smoke');
	same(await client.file('smoke.txt').text(), 'smoke', 'an object read back');

	const get = await fetch(client.presign('smoke.txt', { expiresIn: 60 }));
	same([get.status, await get.text()], [200, 'smoke'], 'a presigned GET');
	const forged = new URL(client.presign('smoke.txt', { expiresIn: 60 }));
	forged.searchParams.set('X-Amz-Signature', '0'.repeat(64));
	const refused = await fetch(forged);
	same(refused.status, 403, 'a presigned GET with a forged signature');
	const put = client.presign('presigned.txt', { method: 'PUT', expiresIn: 60 });
	same((await fetch(put, { method: 'PUT', body: 'put' })).status, 200, 'a presigned PUT');
	same(await client.file('presigned.txt').text(), 'put', 'what the presigned PUT wrote');

	// Bun's client doesn't abort a multipart upload of its own accord, so
	// these requests are signed here.
	const object = `/${s3.bucket}/multipart.bin`;
	const created = await signed(s3, 'POST', object, { uploads: '' });
	const uploadId = (await created.text()).match(/<UploadId>([^<]+)<\/UploadId>/)?.[1];
	same(typeof uploadId, 'string', 'the upload CreateMultipartUpload started');
	const part = await signed(s3, 'PUT', object, { partNumber: '1', uploadId }, new Uint8Array(1024));
	same(part.status, 200, 'UploadPart');
	const uploads = async () => (await (await signed(s3, 'GET', `/${s3.bucket}`, { uploads: '' })).text()).includes(uploadId);
	same(await uploads(), true, 'the upload listed before it was aborted');
	same((await signed(s3, 'DELETE', object, { uploadId })).status, 204, 'AbortMultipartUpload');
	same(await uploads(), false, 'the upload listed after it was aborted');
	same((await signed(s3, 'GET', object, { uploadId })).status, 404, 'the aborted upload\'s parts');
	same(await client.exists('multipart.bin'), false, 'the aborted upload\'s object');
}

// A request signed with AWS Signature Version 4, in its headers.
function signed(s3, method, path, query, body = new Uint8Array(0))
{
	const sha256 = data => createHash('sha256').update(data).digest('hex');
	const hmac = (key, data) => createHmac('sha256', key).update(data).digest();
	const host = new URL(s3.endpoint).host;
	const time = new Date().toISOString().replace(/[-:]|\.\d+/g, '');
	const day = time.slice(0, 8);
	const payload = sha256(body);
	const canonicalQuery = Object.keys(query).sort()
		.map(key => `${encodeURIComponent(key)}=${encodeURIComponent(query[key])}`)
		.join('&');
	const headers = `host:${host}\nx-amz-content-sha256:${payload}\nx-amz-date:${time}\n`;
	const names = 'host;x-amz-content-sha256;x-amz-date';
	const request = [method, path, canonicalQuery, headers, names, payload].join('\n');
	const scope = `${day}/${s3.region}/s3/aws4_request`;
	const toSign = ['AWS4-HMAC-SHA256', time, scope, sha256(request)].join('\n');
	const key = ['s3', 'aws4_request'].reduce(hmac, hmac(hmac('AWS4' + s3.secretAccessKey, day), s3.region));
	const signature = createHmac('sha256', key).update(toSign).digest('hex');
	return fetch(`${s3.endpoint}${path}?${canonicalQuery}`, {
		method,
		body: method === 'GET' || method === 'DELETE' ? undefined : body,
		headers: {
			'x-amz-content-sha256': payload,
			'x-amz-date': time,
			authorization: `AWS4-HMAC-SHA256 Credential=${s3.accessKeyId}/${scope}, SignedHeaders=${names}, Signature=${signature}`
		}
	});
}
