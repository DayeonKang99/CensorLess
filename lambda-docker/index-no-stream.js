const axios = require('axios');
const https = require('https');

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
exports.handler = async (event) => {
  const targetUrl = new URL(event.headers['x-url']);
  // console.log(targetUrl);

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

    // console.log('Forwarding headers:', headers);

    // Prepare the request options for Axios
    const options = {
      method: event.requestContext.http.method,
      url: targetUrl,
      headers: headers,
      data: event.body, // Forward the original request body
      httpsAgent: new https.Agent({ rejectUnauthorized: false }),
      responseType: 'arraybuffer' // IMPORTANT: to handle binary data, use arraybuffer
    };

    // Send the request to the target server using Axios
    const proxyRes = await axios(options);

    // Lambda didn't support chunked encoding
    const responseHeaders = { ...proxyRes.headers };
    delete responseHeaders['transfer-encoding'];

    const isBinary = responseHeaders['content-type'] && !responseHeaders['content-type'].startsWith('text/');
    const responseBody = isBinary ? Buffer.from(proxyRes.data).toString('base64') : Buffer.from(proxyRes.data).toString('utf-8');

    return res = {
      statusCode: proxyRes.status,
      headers: responseHeaders,
      body: responseBody,
      isBase64Encoded: isBinary
    };

  } catch (error) {
    console.error('Proxy request error:', error);

    return {
      statusCode: 500,
      body: 'Internal Server Error'
    };
  }
};

