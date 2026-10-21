/**
 * One authenticated JSON-RPC route served from the plugin's own `webServer`
 * registration.
 *
 * The browser's `ctx.connection.rpc.call(channel, endpoint, payload)` posts a
 * `client-request` envelope to `<channel>/<endpoint>` and expects a
 * `server-response` envelope back. Connection's own channel registry cannot be
 * extended by another plugin (its `register` reads `webServer` from the
 * Connection fiber), so this module speaks that wire format directly and
 * reuses Connection's request admission for the browser cookie and origin
 * fence.
 */

/** Largest accepted request body for this route; the payload is two numbers. */
export const MAX_BODY_BYTES = 64 * 1024

/** Endpoint segments accepted by the wire contract. */
const ENDPOINT_SEGMENT = /^[A-Za-z0-9_$.-]+$/

/**
 * Read one raw request body with a hard byte cap.
 * @param req - node:http request.
 * @returns the body text, or undefined when the stream failed.
 */
async function readBody(req) {
  const chunks = []
  let total = 0
  for await (const chunk of req) {
    const buffer = typeof chunk === 'string' ? Buffer.from(chunk) : chunk
    total += buffer.length
    if (total > MAX_BODY_BYTES) return { tooLarge: true }
    chunks.push(buffer)
  }
  return { text: Buffer.concat(chunks).toString('utf8') }
}

/**
 * Validate the wire envelope.
 * @param value - parsed JSON body.
 * @returns the envelope, or undefined when it is not a client request.
 */
function readEnvelope(value) {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) return undefined
  if (value.type !== 'client-request') return undefined
  if (typeof value.rpcId !== 'string' || value.rpcId === '') return undefined
  if (typeof value.method !== 'string' || value.method === '') return undefined
  return { rpcId: value.rpcId, method: value.method, payload: value.payload }
}

/**
 * Endpoint named by a request URL, relative to the channel prefix.
 * @param url - raw request URL.
 * @param channel - absolute channel prefix such as `/usage-stats`.
 * @returns the endpoint, or undefined when the path is not a valid endpoint.
 */
export function endpointOf(url, channel) {
  let pathname
  try {
    pathname = new URL(String(url), 'http://localhost').pathname
  } catch {
    return undefined
  }
  if (pathname === channel) return undefined
  if (!pathname.startsWith(`${channel}/`)) return undefined
  const endpoint = pathname.slice(channel.length + 1)
  if (endpoint.split('/').some((segment) => segment === '' || segment === '.' || segment === '..' || !ENDPOINT_SEGMENT.test(segment))) return undefined
  return endpoint
}

/**
 * Answer one request with a JSON body.
 * @param res - node:http response.
 * @param status - HTTP status.
 * @param body - JSON-serializable body.
 */
function sendJson(res, status, body) {
  const text = JSON.stringify(body)
  res.writeHead(status, {
    'content-type': 'application/json; charset=utf-8',
    'content-length': String(Buffer.byteLength(text)),
    'cache-control': 'no-store',
  })
  res.end(text)
}

/**
 * Wrap a handler result in the wire's success envelope.
 * @param rpcId - caller correlation id.
 * @param value - endpoint payload.
 * @returns the response body.
 */
function success(rpcId, value) {
  return { type: 'server-response', rpcId, result: { ok: true, value } }
}

/**
 * Wrap a failure in the wire's error envelope.
 * @param rpcId - caller correlation id, or a placeholder for unreadable bodies.
 * @param code - machine code.
 * @param message - human message.
 * @returns the response body.
 */
function failure(rpcId, code, message) {
  return { type: 'server-response', rpcId, result: { ok: false, error: { code, message, details: {} } } }
}

/**
 * Build the node request handler for one channel.
 * @param options - channel path, the admission check, and the endpoint handler.
 * @returns a `(req, res)` handler owned by the caller's `webServer` effect.
 */
export function createRouteHandler(options) {
  const { channel, requestRejection, handle } = options
  return async (req, res) => {
    const rejection = requestRejection(req)
    if (rejection !== undefined) {
      res.writeHead(rejection)
      res.end(rejection === 401 ? 'unauthorized' : 'forbidden')
      return
    }
    if (req.method !== 'POST') {
      res.writeHead(405, { allow: 'POST' })
      res.end()
      return
    }
    const endpoint = endpointOf(req.url, channel)
    if (endpoint === undefined) {
      res.writeHead(404)
      res.end('not found')
      return
    }
    const contentType = String(req.headers['content-type'] || '').split(';', 1)[0].trim().toLowerCase()
    if (contentType !== 'application/json') {
      res.writeHead(415)
      res.end('content type must be application/json')
      return
    }
    const body = await readBody(req)
    if (body.tooLarge === true) {
      res.writeHead(413)
      res.end('request body too large')
      return
    }
    let parsed
    try {
      parsed = JSON.parse(body.text)
    } catch {
      res.writeHead(400)
      res.end('body is not JSON')
      return
    }
    const envelope = readEnvelope(parsed)
    if (envelope === undefined) {
      res.writeHead(400)
      res.end('invalid client-request envelope')
      return
    }
    if (envelope.method !== endpoint) {
      sendJson(res, 200, failure(envelope.rpcId, 'gateway/bad-request', `method ${JSON.stringify(envelope.method)} does not match endpoint ${JSON.stringify(endpoint)}`))
      return
    }
    try {
      const result = await handle(endpoint, envelope.payload)
      if (result !== null && typeof result === 'object' && typeof result.ok === 'boolean') {
        sendJson(res, 200, { type: 'server-response', rpcId: envelope.rpcId, result })
        return
      }
      sendJson(res, 200, success(envelope.rpcId, result))
    } catch (error) {
      sendJson(res, 200, failure(envelope.rpcId, 'usage-stats/failed', error instanceof Error ? error.message : String(error)))
    }
  }
}
