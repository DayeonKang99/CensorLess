const axios = require('axios');
const https = require('https');
const pipeline = require("util").promisify(require("stream").pipeline);
const zlib = require('zlib');
const { Readable } = require('stream');
const AWS = require('aws-sdk');
const lambda = new AWS.Lambda();
const sts = new AWS.STS();

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

        if (event.rawPath && event.rawPath === "/new-url") {
            const identity = await sts.getCallerIdentity().promise();
            const data = await lambda.listTags({
                Resource: `arn:aws:lambda:${process.env.AWS_REGION}:${identity.Account}:function:${process.env.AWS_LAMBDA_FUNCTION_NAME}`
            }).promise();
            console.log("Lambda Tags:", data.Tags['new-lambda-url']); 
            responseStream.write(JSON.stringify({ tags: data.Tags['new-lambda-url'] }));
            responseStream.end();
            return
        }

        const targetUrl = new URL(event.headers['x-url']);
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
            
            // Prepare the request options for Axios
            const options = {
            method: event.requestContext.http.method,
            url: targetUrl,
            headers: headers,
            data: event.body, // Forward the original request body
            httpsAgent: new https.Agent({ rejectUnauthorized: false }),
            responseType: 'arraybuffer' 
            };
            console.log('Proxy request:', options);
            // Send the request to the target server using Axios
            const proxyRes = await axios(options);

            const responseHeaders = { ...proxyRes.headers };
            // delete responseHeaders['transfer-encoding'];
            console.log('responseHeaders:', responseHeaders);

            res = {
            statusCode: proxyRes.status,
            headers: responseHeaders,
            };
            console.log('Proxy response body:', proxyRes.data);
            console.log('Proxy response:', res);

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
