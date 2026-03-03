import assert from 'assert';
import * as net from 'net';
import * as url from 'url';
import * as http from 'http';
import * as https from 'https';
import * as os from 'os';
import * as zlib from 'zlib';
import pkg from './pkg';

import createDebug from 'debug';
import { KeyObject } from 'crypto';

// log levels
const debug = {
	request: createDebug('proxy ← ← ←'),
	response: createDebug('proxy → → →'),
	proxyRequest: createDebug('proxy ↑ ↑ ↑'),
	proxyResponse: createDebug('proxy ↓ ↓ ↓'),
};

// hostname
const hostname = os.hostname();
let lambda_url = '';//

export interface ProxyServer extends http.Server {
	authenticate?: (req: http.IncomingMessage) => boolean | Promise<boolean>;
	localAddress?: string;
}

/**
 * Sets up an `http.Server` or `https.Server` instance with the necessary
 * "request" and "connect" event listeners in order to make the server act
 * as an HTTP proxy.
 */
export function createProxy(server?: http.Server): ProxyServer {
	if (!server) server = http.createServer();
	server.on('request', onrequest);
	// server.on('connect', onconnect);
	return server;
}

/**
 * 13.5.1 End-to-end and Hop-by-hop Headers
 *
 * Hop-by-hop headers must be removed by the proxy before passing it on to the
 * next endpoint. Per-request basis hop-by-hop headers MUST be listed in a
 * Connection header, (section 14.10) to be introduced into HTTP/1.1 (or later).
 */
const hopByHopHeaders = [
	'Connection',
	'Keep-Alive',
	'Proxy-Authenticate',
	'Proxy-Authorization',
	'TE',
	'Trailers',
	'Transfer-Encoding',
	'Upgrade',
];

// create a case-insensitive RegExp to match "hop by hop" headers
const isHopByHop = new RegExp('^(' + hopByHopHeaders.join('|') + ')$', 'i');

const AcceptHeaders = [
	'Accept',
	'Accept-Encoding',
	'Accept-Language',
	'Content-Type',
	'Last-Modified',
	'Expires',
	'Date',
	'Age',
	'Accept-Ranges',
	'Server',
	'User-Agent',
	'Host',
	'If-Modified-Since',
	'If-None-Match',
	'Cookie',
	'Cache-Control',
	'Access-Control-Allow-Methods',
	'Access-Control-Allow-Credentials',
	'Very',
	'ETag',
	'Pragma',
	'Accept-Ranges',
	'Referrer-Policy',
	'Permissions-Policy',
	'X-Frame-Options',
	'X-Content-Type-Options',
	'P3P',
	'Mime-Version',
	//'Strict-Transport-Security',
];

// create a case-insensitive RegExp to match "hop by hop" headers
const isAcceptHeaders = new RegExp('^(' + AcceptHeaders.join('|') + ')$', 'i');

const ReplaceContents = [ 'text', 'javascript', 'css', 'json' ];

function isReplaceContent(value: string): boolean {
	return ReplaceContents.some(x => value.includes(x));
}

const pollingInterval = 10000;
const getNextURL = () => {
	// console.log('called this function to get next url');
	https.get(new URL(lambda_url+'new-url'), (response) => {
		// console.log('sent poll');
		let data = '';
		response.on('data', (chunk) => {
			data += chunk;
	});
	response.on('end', () => {
		const parsedData = JSON.parse(data);
		if (String(parsedData.tags).includes('https:')) {
			lambda_url = parsedData.tags
			console.log('Change Lambda URL to ', lambda_url);
		}
	});
	});
}
setInterval(getNextURL, pollingInterval);
getNextURL(); //

/**
 * Iterator function for the request/response's "headers".
 */
function* eachHeader(obj: http.IncomingMessage) {
	// every even entry is a "key", every odd entry is a "value"
	let key: string | null = null;
	for (const v of obj.rawHeaders) {
		if (key === null) {
			key = v;
		} else {
			yield [key, v];
			key = null;
		}
	}
}

/**
 * HTTP GET/POST/DELETE/PUT, etc. proxy requests.
 */
async function onrequest(
	this: ProxyServer,
	req: http.IncomingMessage,
	res: http.ServerResponse
) {
	debug.request('%s %s HTTP/%s ', req.method, req.url, req.httpVersion);
	const socket = req.socket;

	// pause the socket during authentication so no data is lost
	socket.pause();

	try {
		const success = await authenticate(this, req);
		if (!success) return requestAuthorization(req, res);
	} catch (_err: unknown) {
		const err = _err as Error;
		// an error occurred during login!
		res.writeHead(500);
		res.end((err.stack || err.message || err) + '\n');
		return;
	}

	socket.resume();
	console.log(lambda_url);
	let lambda_parsed = url.parse(lambda_url); //'https://7we6n6kmkv7uc6p7zvvjntkmqa0zelbp.lambda-url.us-east-1.on.aws/'); //'https://vbqy7yl7chob4inplz437g4are0jnqll.lambda-url.us-east-1.on.aws/'); // 
	const parsed = url.parse(req.url || '/');

	// setup outbound proxy request HTTP headers
	const headers: http.OutgoingHttpHeaders = {};

	for (let [key, value] of eachHeader(req)) { //const [key, value]
		let keyLowerCase = key.toLowerCase();

		debug.request('Request Header: %o', [key, value]);
		if (isAcceptHeaders.test(key)) {
			if (keyLowerCase === 'host')
				value = lambda_parsed.host || '';
			const v = headers[key] as string;
			if (Array.isArray(v)) {
				v.push(value);
			} else if (null != v) {
				headers[key] = [v, value];
			} else {
				headers[key] = value;
			}
		}
		else if (keyLowerCase === 'strict-transport-security') {
			let knockout = 'max-age=0';

			const v = headers[key] as string;
			if (Array.isArray(v)) {
				v.push(knockout);
			} else if (null != v) {
				headers[key] = [v, knockout];
			} else {
				headers[key] = knockout;
			}
		}
		else if ((keyLowerCase === 'referer') || (keyLowerCase === 'origin')) 
			headers[key] = value.replace('http:', 'https:');
	}

	// custom `http.Agent` support, set `server.agent`
	//let agent = server.agent;
	//if (null != agent) {
	//	debug.proxyRequest(
	//		'setting custom `http.Agent` option for proxy request: %s',
	//		agent
	//	);
	//	parsed.agent = agent;
	//	agent = null;
	//}

	//if (!parsed.port) {
	//	// default the port number if not specified, for >= node v0.11.6...
	//	// https://github.com/joyent/node/issues/6199
	//	parsed.port = 80;
	//}

	if (parsed.protocol !== 'http:') {
		// only "http://" is supported, "https://" should use CONNECT method
		res.writeHead(400);
		res.end(
			`Only "http:" protocol prefix is supported (got: "${parsed.protocol}")\n`
		);
		return;
	}

	// headers['X-host'] = parsed.host || '';
	// headers['X-hostname'] = parsed.hostname || '';
	// headers['X-path'] = parsed.path || '';
	// headers['X-port'] = ((parsed.port == null) || (parsed.port === "80")) ? 443: parsed.port;
	headers['target-url'] = (req.url || '').replace('http:', 'https:');
	// console.log(req.url);

	let gotResponse = false;

	debug.response('Request Headers: %o', headers);
	const proxyReq = https.request({
		hostname: lambda_parsed.hostname,
		path: lambda_parsed.path,
		port: ((lambda_parsed.port == null) || (lambda_parsed.port === "80")) ? 443: lambda_parsed.port,
		method: req.method, //..? this should be about lambda too?
		headers,
		agent: new https.Agent({ keepAlive: false }),
	});
	debug.proxyRequest('%s %s HTTP/1.1 ', proxyReq.method, proxyReq.path);
	// console.log(JSON.parse(JSON.stringify(proxyReq)));
	// console.log(proxyReq);

	proxyReq.on('response', function (proxyRes) {
		debug.proxyResponse('HTTP/1.1 %s', proxyRes.statusCode);
		gotResponse = true;
		let replaceBody = false;
		// console.log(proxyRes);

		let stream: http.IncomingMessage | zlib.Gunzip = proxyRes;
		const headers: http.OutgoingHttpHeaders = {};
		for (let [key, value] of eachHeader(proxyRes)) { //const [key, value]
			let keyLowerCase = key.toLowerCase();

			debug.proxyResponse('Proxy Response Header: "%s: %s"', key, value);
			if (isAcceptHeaders.test(key)) {
				if (keyLowerCase == 'strict-transport-security') //
					value = 'max-age=0';
				const v = headers[key] as string;
				if (Array.isArray(v)) {
					v.push(value);
				} else if (null != v) {
					headers[key] = [v, value];
				} else {
					headers[key] = value;
				}
			}
			// CORS 오류 발생은 'Access-Control-Allow-Origin'이 처리되지 않아 발생됨
			else if ((keyLowerCase == 'location') || (keyLowerCase === 'access-control-allow-origin'))
				headers[key] = value.replace('https:', 'http:');
			else if (keyLowerCase == 'set-cookie') {
				let cookie = value.replace(' Secure;', '').replace('; Secure', '');

				const v = headers[key] as string;
				if (Array.isArray(v)) {
					v.push(cookie);
				} else if (null != v) {
					headers[key] = [v, cookie];
				} else {
					headers[key] = cookie;
				}
			}
			else if (keyLowerCase === 'content-encoding') {
				if (value === 'gzip') stream = proxyRes.pipe(zlib.createGunzip());
			}
			// else if (keyLowerCase == 'x-lambda-url') {
			// 	lambda_url = value;
			// 	console.log('change lambda url: ', lambda_url);
			// }
			if (keyLowerCase == 'content-type') replaceBody = isReplaceContent(value);
		}

		debug.response('HTTP/1.1 %s', proxyRes.statusCode, replaceBody);
		debug.response('Response Headers: %o', headers);

		headers['Connection'] = 'close';
		res.writeHead(proxyRes.statusCode || 200, headers);

		if (replaceBody == false) {
			proxyRes.pipe(res);
		}
		else {
			let content = '';
			stream.setEncoding('utf-8');
			stream.on('data', (chunk: string) => {
				content += chunk;
			})
			stream.on('end', () => {
				res.end(content.replace(/https:\/\//gi, 'http://'));
			})
		}
		res.on('finish', onfinish);
	});

	proxyReq.on('error', function (err: NodeJS.ErrnoException) {
		debug.proxyResponse(
			'proxy HTTP request "error" event\n%s',
			err.stack || err
		);
		cleanup();
		if (gotResponse) {
			debug.response(
				'already sent a response, just destroying the socket...'
			);
			socket.destroy();
		} else if ('ENOTFOUND' == err.code) {
			debug.response('HTTP/1.1 404 Not Found');
			res.writeHead(404);
			res.end();
		} else {
			debug.response('HTTP/1.1 500 Internal Server Error');
			res.writeHead(500);
			res.end();
		}
	});

	// if the client closes the connection prematurely,
	// then close the upstream socket
	function onclose() {
		debug.request(
			'client socket "close" event, aborting HTTP request to "%s"',
			req.url
		);
		proxyReq.abort();
		cleanup();
	}
	socket.on('close', onclose);

	function onfinish() {
		debug.response('"finish" event');
		cleanup();
	}

	function cleanup() {
		debug.response('cleanup');
		socket.removeListener('close', onclose);
		res.removeListener('finish', onfinish);
	}

	req.pipe(proxyReq);
}


// const localPort = 3000;
// const redirectServer = http.createServer((req, res) => {
// 	console.log('received!');
// });
// redirectServer.listen(localPort, () => {
// 	console.log('listening');
// });

/**
 * HTTP CONNECT proxy requests.
 */
 async function onconnect(
	this: ProxyServer,
	req: http.IncomingMessage,
	socket: net.Socket,
	head: Buffer
) {
	debug.request('%s %s HTTP/%s ', req.method, req.url, req.httpVersion);
	assert(
		!head || 0 == head.length,
		'"head" should be empty for proxy requests'
	);

	let res: http.ServerResponse | null;
	let gotResponse = false;

	
	// if (req.method !== 'CONNECT') {
	// 	return socket.end('HTTP/1.1 405 Method Not Allowed\r\n\r\n');
	// }

	// const target = req.url;
	// const [hostname, port] = target?.split(':') ?? [];
	// // console.log('hostname: ', hostname, '| port: ', port);
	// const serverSocket = net.connect(localPort, 'localhost', () => {
	// 	socket.write('HTTP/1.1 200 Connection Established\r\n\r\n');
	// 	socket.pipe(serverSocket);
	// 	serverSocket.pipe(socket);
	// })
	
	// // define request socket event listeners
	// socket.on('close', function onclientclose() {
	// 	debug.request('HTTP request %s socket "close" event', req.url);
	// });

	// socket.on('end', function onclientend() {
	// 	debug.request('HTTP request %s socket "end" event', req.url);
	// });

	// socket.on('error', function onclienterror(err) {
	// 	debug.request(
	// 		'HTTP request %s socket "error" event:\n%s',
	// 		req.url,
	// 		err.stack || err
	// 	);
	// });

	/*
	// define target socket event listeners
	function ontargetclose() {
		debug.proxyResponse('proxy target %s "close" event', req.url);
		socket.destroy();
	}

	function ontargetend() {
		debug.proxyResponse('proxy target %s "end" event', req.url);
	}

	function ontargeterror(err: NodeJS.ErrnoException) {
		debug.proxyResponse(
			'proxy target %s "error" event:\n%s',
			req.url,
			err.stack || err
		);
		if (gotResponse) {
			debug.response(
				'already sent a response, just destroying the socket...'
			);
			socket.destroy();
		} else if (err.code === 'ENOTFOUND') {
			debug.response('HTTP/1.1 404 Not Found');
			if (res) {
				res.writeHead(404);
				res.end();
			}
		} else {
			debug.response('HTTP/1.1 500 Internal Server Error');
			if (res) {
				res.writeHead(500);
				res.end();
			}
		}
	}

	function ontargetconnect() {
		debug.proxyResponse('proxy target %s "connect" event', req.url);
		debug.response('HTTP/1.1 200 Connection established');
		gotResponse = true;

		if (res) {
			res.removeListener('finish', onfinish);

			res.writeHead(200, 'Connection established');
			res.flushHeaders();

			// relinquish control of the `socket` from the ServerResponse instance
			res.detachSocket(socket);

			// nullify the ServerResponse object, so that it can be cleaned
			// up before this socket proxying is completed
			res = null;
		}

		socket.pipe(target);
		target.pipe(socket);
	}

	// create the `res` instance for this request since Node.js
	// doesn't provide us with one :(
	res = new http.ServerResponse(req);
	res.shouldKeepAlive = false;
	res.chunkedEncoding = false;
	res.useChunkedEncodingByDefault = false;
	res.assignSocket(socket);

	// called for the ServerResponse's "finish" event
	// XXX: normally, node's "http" module has a "finish" event listener that would
	// take care of closing the socket once the HTTP response has completed, but
	// since we're making this ServerResponse instance manually, that event handler
	// never gets hooked up, so we must manually close the socket...
	function onfinish() {
		debug.response('response "finish" event');
		if (res) {
			res.detachSocket(socket);
		}
		socket.end();
	}
	res.once('finish', onfinish);

	// pause the socket during authentication so no data is lost
	socket.pause();

	try {
		const success = await authenticate(this, req);
		if (!success) return requestAuthorization(req, res);
	} catch (_err) {
		const err = _err as Error;
		// an error occurred during login!
		res.writeHead(500);
		res.end((err.stack || err.message || err) + '\n');
		return;
	}

	socket.resume();

	if (!req.url) {
		throw new TypeError('No "url" provided');
	}

	// `req.url` should look like "example.com:443"
	const lastColon = req.url.lastIndexOf(':');
	const host = req.url.substring(0, lastColon);
	const port = parseInt(req.url.substring(lastColon + 1), 10);
	const localAddress = this.localAddress;
	const opts = { host: host.replace(/^\[|\]$/g, ''), port, localAddress };

	debug.proxyRequest('connecting to proxy target %o', opts);
	const target = net.connect(opts);
	target.on('connect', ontargetconnect);
	target.on('close', ontargetclose);
	target.on('error', ontargeterror);
	target.on('end', ontargetend);
	*/
} 

/**
 * Checks `Proxy-Authorization` request headers. Same logic applied to CONNECT
 * requests as well as regular HTTP requests.
 */
async function authenticate(server: ProxyServer, req: http.IncomingMessage) {
	if (typeof server.authenticate === 'function') {
		debug.request('authenticating request "%s %s"', req.method, req.url);
		return server.authenticate(req);
	}
	// no `server.authenticate()` function, so just allow the request
	return true;
}

/**
 * Sends a "407 Proxy Authentication Required" HTTP response to the `socket`.
 */
function requestAuthorization(
	req: http.IncomingMessage,
	res: http.ServerResponse
) {
	// request Basic proxy authorization
	debug.response(
		'requesting proxy authorization for "%s %s"',
		req.method,
		req.url
	);

	// TODO: make "realm" and "type" (Basic) be configurable...
	const realm = 'proxy';

	const headers = {
		'Proxy-Authenticate': 'Basic realm="' + realm + '"',
	};
	res.writeHead(407, headers);
	res.end('Proxy authorization required');
}
