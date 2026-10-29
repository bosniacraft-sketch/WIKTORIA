#!/usr/bin/env python3
"""Look at the interface without the game: serves apps/web and answers the page's questions from
made-up fixtures, so a window can be laid out and photographed anywhere.

    python apps/web/dev/mock.py [--port 8099]

GET /api/<a>/<b>?...  -> fixtures/<a>_<b>.json (or {} when there is none)
POST /api/...         -> {"ok":true,"text":"mock"}
GET /map.bmp          -> a plain sea-coloured picture of the asked width

Everything in fixtures/ is invented (countries "Country A", "Country B"...), never the original's text.
"""
import json, os, struct, sys
from http.server import ThreadingHTTPServer, BaseHTTPRequestHandler
from urllib.parse import urlparse, parse_qs

HERE = os.path.dirname(os.path.abspath(__file__))
WEB = os.path.dirname(HERE)
FIX = os.path.join(HERE, 'fixtures')
TYPES = {'.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.svg': 'image/svg+xml',
         '.png': 'image/png', '.json': 'application/json', '.woff2': 'font/woff2'}


def bmp(w, h, rgb=(52, 74, 96)):
    row = bytes([rgb[2], rgb[1], rgb[0]]) * w
    row += b'\0' * ((4 - len(row) % 4) % 4)
    data = row * h
    head = b'BM' + struct.pack('<IHHI', 54 + len(data), 0, 0, 54)
    info = struct.pack('<IiiHHIIiiII', 40, w, h, 1, 24, 0, len(data), 2835, 2835, 0, 0)
    return head + info + data


class H(BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def send(self, code, body, ctype):
        self.send_response(code)
        self.send_header('Content-Type', ctype)
        self.send_header('Content-Length', str(len(body)))
        self.send_header('Cache-Control', 'no-store')
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        u = urlparse(self.path)
        if u.path.startswith('/api/'):
            f = os.path.join(FIX, u.path[1:].replace('/', '_') + '.json')
            # MOCK_EXTRA: a folder of answers saved from `wiktoria-cli serve` (kept out of the repo), tried first.
            extra = os.path.join(os.environ.get('MOCK_EXTRA', '/nonexistent'), u.path[1:].replace('/', '_') + '.json')
            if os.path.exists(extra):
                f = extra
            body = open(f, 'rb').read() if os.path.exists(f) else b'{}'
            return self.send(200, body, 'application/json')
        # MOCK_MAP / MOCK_PAPER: real pictures saved from `wiktoria-cli serve`, to see the map look.
        if u.path == '/map.bmp' and os.environ.get('MOCK_MAP'):
            return self.send(200, open(os.environ['MOCK_MAP'], 'rb').read(), 'image/bmp')
        if u.path == '/map/paper.bmp':
            if not os.environ.get('MOCK_PAPER'):
                return self.send(404, b'not found', 'text/plain')
            return self.send(200, open(os.environ['MOCK_PAPER'], 'rb').read(), 'image/bmp')
        if u.path == '/map.bmp':
            w = int(parse_qs(u.query).get('width', ['1024'])[0])
            return self.send(200, bmp(w, max(1, w * 2160 // 5616)), 'image/bmp')
        rel = 'index.html' if u.path in ('', '/') else u.path.lstrip('/')
        p = os.path.normpath(os.path.join(WEB, rel))
        if not p.startswith(WEB) or not os.path.isfile(p):
            return self.send(404, b'not found', 'text/plain')
        self.send(200, open(p, 'rb').read(), TYPES.get(os.path.splitext(p)[1], 'application/octet-stream'))

    def do_POST(self):
        self.send(200, json.dumps({'ok': True, 'text': 'mock'}).encode(), 'application/json')


if __name__ == '__main__':
    port = int(sys.argv[sys.argv.index('--port') + 1]) if '--port' in sys.argv else 8099
    print(f'mock interface on http://127.0.0.1:{port}/')
    ThreadingHTTPServer(('127.0.0.1', port), H).serve_forever()
