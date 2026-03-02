const axios = require('axios');
const https = require('https');
const pipeline = require("util").promisify(require("stream").pipeline);
const zlib = require('zlib');
const { Readable } = require('stream');

const acceptHeaders = [
  'content-length',
  'content-type',
  'accept-language',
  'accept',
  'accept-encoding',
  'user-agent'
];

const isAcceptHeaders = new RegExp('^(' + acceptHeaders.join('|') + ')$', 'i');

// Main Lambda function handler
exports.handler = awslambda.streamifyResponse(
    async (event, responseStream, context) => {
        const targetUrl = new URL(event.headers['target-url']);
        console.log(targetUrl);

        try {
            const headers = {};
            for (const [key, value] of Object.entries(event.headers)) {
            if (isAcceptHeaders.test(key)) {
                const existingValue = headers[key];
                if (Array.isArray(existingValue)) {
                existingValue.push(value);
                } else if (existingValue != null) {
                headers[key] = [existingValue, value];
                } else {
                headers[key] = value;
                }
            }
            }

            if (targetUrl.origin.includes("facebook")){
            headers['user-agent'] = 'facebookexternalhit/1.1 (+http://www.facebook.com/externalhit_uatext.php)'
            };
            
            const options = {
            method: event.requestContext.http.method,
            url: targetUrl,
            headers: headers,
            data: event.body, // Forward the original request body
            httpsAgent: new https.Agent({ rejectUnauthorized: false }),
            responseType: 'arraybuffer' 
            };
            const proxyRes = await axios(options);

            const responseHeaders = { ...proxyRes.headers };
            // delete responseHeaders['transfer-encoding'];

            res = {
            statusCode: proxyRes.status,
            headers: responseHeaders,
            };
            responseBody = Readable.from(Buffer.from(proxyRes.data));
            responseStream = awslambda.HttpResponseStream.from(responseStream, res);
            console.log('responseStream:', responseStream);
            await pipeline(
                responseBody,
                responseStream,
            );

        } catch (error) {
            console.error('Proxy request error:', error);

            return {
            statusCode: 500,
            body: 'Internal Server Error'
            };
        }
    }
)
