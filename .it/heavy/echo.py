"""The upstream echo of the heavy suite (RFC 0016 §6.1): an HTTPS server that
stands where an apiserver would and writes every request it receives, one
JSON line on stdout, so a test reads from `kubectl logs` what the proxy
relayed upstream.

It answers like an apiserver only where the proxy needs it:
- a `SelfSubjectReview` echoes the `X-Remote-*` identity back, so the
  FrontProxy trust probe concludes `Trusted` (RFC 0014 decision 60);
- a WebSocket upgrade is accepted with the first subprotocol offered, then
  closed: the handshake is what point 16 reads, not the stream;
- anything else is a `200` with a JSON body.

Standard library only: the image is a pinned python (`.it/images.lock`), the
script and the certificate are mounted by the test.
"""

import base64
import hashlib
import json
import ssl
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import unquote

WS_GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"


def identity(headers):
    """The userInfo an apiserver in front-proxy mode would build."""
    user = headers.get("X-Remote-User")
    if user is None:
        return None
    extra = {}
    for name, value in headers.items():
        if name.lower().startswith("x-remote-extra-"):
            key = unquote(name[len("x-remote-extra-"):]).lower()
            extra.setdefault(key, []).append(value)
    return {
        "username": user,
        "groups": headers.get_all("X-Remote-Group") or [],
        "extra": extra,
    }


class Echo(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *_):
        pass

    def record(self):
        line = {
            "method": self.command,
            "path": self.path,
            "headers": [[k, v] for k, v in self.headers.items()],
        }
        sys.stdout.write(json.dumps(line) + "\n")
        sys.stdout.flush()

    def reply(self, status, body):
        data = json.dumps(body).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def handle_any(self):
        length = int(self.headers.get("Content-Length") or 0)
        if length:
            self.rfile.read(length)
        self.record()
        if self.headers.get("Upgrade", "").lower() == "websocket":
            key = self.headers.get("Sec-WebSocket-Key", "")
            accept = base64.b64encode(hashlib.sha1((key + WS_GUID).encode()).digest()).decode()
            offered = [p.strip() for p in self.headers.get("Sec-WebSocket-Protocol", "").split(",") if p.strip()]
            self.send_response(101)
            self.send_header("Upgrade", "websocket")
            self.send_header("Connection", "Upgrade")
            self.send_header("Sec-WebSocket-Accept", accept)
            if offered:
                self.send_header("Sec-WebSocket-Protocol", offered[0])
            self.end_headers()
            self.close_connection = True
            return
        if self.command == "POST" and self.path.endswith("/selfsubjectreviews"):
            user = identity(self.headers)
            if user is None:
                self.reply(401, {"kind": "Status", "status": "Failure", "code": 401})
            else:
                self.reply(201, {"kind": "SelfSubjectReview", "status": {"userInfo": user}})
            return
        self.reply(200, {"kind": "Status", "status": "Success", "echo": self.path})

    do_GET = do_POST = do_PUT = do_PATCH = do_DELETE = do_HEAD = handle_any


def main():
    server = ThreadingHTTPServer(("0.0.0.0", 8443), Echo)
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain("/tls/tls.crt", "/tls/tls.key")
    server.socket = context.wrap_socket(server.socket, server_side=True)
    print(json.dumps({"listening": 8443}), flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
