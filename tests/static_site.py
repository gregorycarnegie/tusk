"""Smoke-test the built dist directory in Chrome: python tests/static_site.py.

Uses only Python's standard library and an installed Chrome/chromedriver.
The fake HID and clipboard keep this test independent of hardware/permissions.
"""
import base64
import csv
import functools
import http.server
import io
import json
import os
import shutil
import socket
import subprocess
import tempfile
import threading
import time
import urllib.request
from pathlib import Path

FAKE_BROWSER = r"""
window.errors = [];
addEventListener('error', e => errors.push(e.message));
addEventListener('unhandledrejection', e => errors.push(String(e.reason)));
window.reader = {
    vendorId: 0x1071, opened: false, card: true, unplugged: false,
    async open() { this.opened = true; },
    async sendReport(id, bytes) {
        if (this.unplugged) throw new DOMException('unplugged', 'NotFoundError');
        const key = new TextEncoder().encode('Elephant');
        const op = bytes[3] ^ ((bytes[2] & 128) ? key[bytes[2] % 8] : 0);
        if (op !== 0xd7 && op !== 0x14) return;
        // Captured Mifare frame from src/protocol.rs; empty reads use a valid ACK.
        let reply = new Uint8Array([
            2,37,165,113,53,9,5,85,101,112,104,97,110,116,69,108,
            101,112,104,97,110,116,69,108,101,112,104,97,110,116,69,108,
            101,112,104,97,249
        ]);
        if (op !== 0xd7 || !this.card) {
            reply = new Uint8Array(37);
            reply.set([2,37,0,16]);
            reply[36] = 255 - (reply.slice(0,36).reduce((a,b) => a+b,0) % 256);
        }
        if (op === 0xd7 && this.card && this.uid) {
            reply = new Uint8Array(37);
            reply.set([2,37,0,16]);
            reply.set(this.uid, 4);
            reply[36] = 255 - (reply.slice(0,36).reduce((a,b) => a+b,0) % 256);
        }
        setTimeout(() => this.oninputreport?.({data: new DataView(reply.buffer)}), 5);
    }
};
window.cancelPicker = true;
Object.defineProperty(navigator, 'hid', {configurable: true, value: {
    async getDevices() { return localStorage.authorized ? [reader] : []; },
    async requestDevice() {
        if (cancelPicker) return [];
        localStorage.authorized = 'true';
        return [reader];
    }
}});
Object.defineProperty(navigator, 'clipboard', {value: {
    async writeText(text) { window.copied = text; }
}});
// A Net2 Local API at https://net2.test: users 7 and 12 have no portrait or
// card, 8 has both. Every request is recorded in net2.calls.
window.net2 = {calls: [], cards: {8: ['11111111']}, images: {}};
const realFetch = window.fetch.bind(window);
window.fetch = async (url, init = {}) => {
    const api = 'https://net2.test/api/v1';
    if (!String(url).startsWith(api)) return realFetch(url, init);
    const path = String(url).slice(api.length), method = init.method || 'GET';
    const auth = init.headers && init.headers.get('Authorization');
    const kind = init.headers && init.headers.get('Content-Type');
    const body = !init.body ? null : kind === 'application/json'
        ? JSON.parse(init.body) : Object.fromEntries(new URLSearchParams(init.body));
    net2.calls.push({method, path, body, auth, kind});
    const reply = (status, data) => new Response(data === undefined ? null : JSON.stringify(data), {status});
    if (path === '/authorization/tokens') {
        return body.password === 'right &=%' ? reply(200, {access_token: 'T0K'}) : reply(400, {message: 'invalid_client'});
    }
    if (auth !== 'Bearer T0K') return reply(401, {Message: 'Authorization has been denied'});
    const users = {
        7: {id: 7, firstName: 'Ada', lastName: 'Lovelace', hasImage: false},
        8: {id: 8, firstName: 'Jane', lastName: 'Doe', hasImage: true},
        12: {id: 12, firstName: 'John', lastName: 'Roe', hasImage: false},
    };
    let m;
    if (path === '/departments') return reply(200, [{id: 3, name: 'Year 7'}]);
    if (path === '/departments/3/users') return reply(200, [users[8], users[12]]);
    if ((m = path.match(/^\/users\/(\d+)$/))) return users[m[1]] ? reply(200, users[m[1]]) : reply(404);
    if ((m = path.match(/^\/users\/(\d+)\/image$/)) && method === 'PUT') {
        net2.images[m[1]] = body.base64Data;
        return reply(204);
    }
    if ((m = path.match(/^\/users\/(\d+)\/tokens$/))) {
        const cards = net2.cards[m[1]] = net2.cards[m[1]] || [];
        if (method === 'POST') { cards.push(body.tokenValue); return reply(201, body); }
        return reply(200, cards.map(tokenValue => ({tokenType: 'ProxCard', tokenValue, isLost: false})));
    }
    return reply(404);
};
"""


def main():
    root = Path(__file__).resolve().parents[1]
    for name in ['index.html', 'tusk.js', 'tusk_bg.wasm', '.nojekyll']:
        assert (root / 'dist' / name).is_file(), f'Build dist first: missing {name}'
    with tempfile.TemporaryDirectory() as directory:
        shutil.copytree(root / 'dist', Path(directory) / 'tusk')
        handler = functools.partial(http.server.SimpleHTTPRequestHandler, directory=directory)
        server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), handler)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        with socket.socket() as sock:
            sock.bind(('127.0.0.1', 0))
            port = sock.getsockname()[1]
        driver = subprocess.Popen(
            [os.environ.get('CHROMEDRIVER', 'chromedriver'), f'--port={port}'],
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
            creationflags=getattr(subprocess, 'CREATE_NO_WINDOW', 0),
        )
        session = None

        def request(method, path, data=None):
            body = None if data is None else json.dumps(data).encode()
            req = urllib.request.Request(f'http://127.0.0.1:{port}{path}', data=body,
                                         method=method, headers={'Content-Type': 'application/json'})
            with urllib.request.urlopen(req, timeout=20) as response:
                return json.load(response)['value']

        def command(path, data):
            return request('POST', f'/session/{session}/{path}', data)

        def js(script):
            return command('execute/sync', {'script': script, 'args': []})

        def wait(expression):
            deadline = time.monotonic() + 8
            while time.monotonic() < deadline:
                if js('return ' + expression):
                    return
                time.sleep(.05)
            raise AssertionError(f'Timed out: {expression}; errors: {js("return window.errors")}')

        def upload(name, content):
            js(f"""
                const transfer = new DataTransfer();
                transfer.items.add(new File([{json.dumps(content)}], {json.dumps(name)}, {{type: 'text/csv'}}));
                const input = document.getElementById('batch-file');
                input.files = transfer.files;
                input.dispatchEvent(new Event('change', {{bubbles: true}}));
            """)
            wait("!document.getElementById('batch-file').disabled")

        def downloaded(name):
            path = Path(directory) / 'downloads' / name
            deadline = time.monotonic() + 8
            while time.monotonic() < deadline:
                if path.exists():
                    return list(csv.reader(io.StringIO(path.read_text(encoding='utf-8-sig'))))
                time.sleep(.05)
            raise AssertionError(f'Download missing: {path}')

        try:
            for _ in range(100):
                try:
                    request('GET', '/status')
                    break
                except OSError:
                    if driver.poll() is not None:
                        raise RuntimeError('chromedriver exited before starting')
                    time.sleep(.05)
            session = request('POST', '/session', {'capabilities': {'alwaysMatch': {
                'browserName': 'chrome',
                'goog:chromeOptions': {'args': ['--headless=new', '--no-sandbox', '--disable-dev-shm-usage']},
            }}})['sessionId']
            command('goog/cdp/execute', {'cmd': 'Page.addScriptToEvaluateOnNewDocument',
                                       'params': {'source': FAKE_BROWSER}})
            command('goog/cdp/execute', {'cmd': 'Browser.setDownloadBehavior', 'params': {
                'behavior': 'allow', 'downloadPath': str(Path(directory) / 'downloads'),
            }})
            url = f'http://127.0.0.1:{server.server_port}/tusk/'
            command('url', {'url': url})
            wait("document.getElementById('message').textContent === 'Not connected'")
            assert js("return document.getElementById('copy-number').disabled")
            js("document.getElementById('connect').click()")
            wait("document.getElementById('message').textContent === 'No reader selected'")
            js("cancelPicker = false; document.getElementById('connect').click()")
            wait("document.getElementById('number').textContent === '34935097'")
            assert js("return document.getElementById('hex').textContent === '5B7D4039'")
            for control, expected in [('copy-number', '34935097'), ('copy-hex', '5B7D4039')]:
                js(f"document.getElementById('{control}').click()")
                wait(f"window.copied === '{expected}' && document.getElementById('{control}').dataset.copied === 'true'")
            js("document.getElementById('theme').click()")
            theme = js('return document.documentElement.dataset.theme')
            command('refresh', {})
            wait("document.getElementById('number').textContent === '34935097'")
            assert js('return document.documentElement.dataset.theme') == theme
            js('reader.card = false')
            wait("document.getElementById('number').textContent === '--------'")
            assert js("return document.getElementById('copy-number').disabled && document.getElementById('hex').hidden")

            # The batch uses the actual file picker handler and download in the built site.
            js("location.hash = 'batch'")
            wait("!document.getElementById('batch-tool').hidden")
            source_rows = [
                ['Surname', 'First name', 'Card Number', 'Notes', 'User ID'],
                ['Doe', 'John', '', 'Comma, quote " and\nnewline', '001'],
                ['Dawkins', 'Jane', '', '', '002', ''],
                ['<img src=x onerror=alert(1)>', 'Éva', '00012345', 'keep', '003'],
                ['Student', 'Absent', '', 'skip', '004'],
            ]
            source = io.StringIO(newline='')
            csv.writer(source).writerows(source_rows)
            upload('students.csv', source.getvalue())
            assert js("return document.getElementById('batch-format').value === 'decimal'")
            assert js("return document.getElementById('batch-prompt').textContent === 'Next: John Doe'")
            assert js("return document.querySelectorAll('#batch-rows img').length === 0")
            js("document.getElementById('batch-start').click(); reader.card = true")
            wait("document.getElementById('batch-progress').textContent.includes('1 assigned')")
            assert js("return !dispatchEvent(new Event('beforeunload', {cancelable: true}))")
            time.sleep(.8)  # A held card must not populate subsequent people.
            assert js("return document.getElementById('batch-prompt').textContent.includes('Jane Dawkins')")
            js('reader.card = false')
            wait("document.getElementById('number').textContent === '--------'")
            js('reader.card = true')
            wait("document.getElementById('batch-notice').textContent.includes('already belongs to John Doe')")
            js('reader.uid = [0x5b,0x7d,0x40,0x3b]')
            wait("document.getElementById('batch-progress').textContent.includes('2 assigned')")
            # Check the batch controls fit a phone and retain screenshots for visual review.
            for width, label in [(1100, 'desktop'), (390, 'mobile')]:
                command('goog/cdp/execute', {'cmd': 'Emulation.setDeviceMetricsOverride', 'params': {
                    'width': width, 'height': 900, 'deviceScaleFactor': 1, 'mobile': False,
                }})
                js("document.getElementById('batch-session').scrollIntoView()")
                assert js('return document.documentElement.scrollWidth <= innerWidth')
                (root / 'target' / f'batch-{label}.png').write_bytes(base64.b64decode(request('GET', f'/session/{session}/screenshot')))
            command('goog/cdp/execute', {'cmd': 'Emulation.clearDeviceMetricsOverride', 'params': {}})
            js("document.getElementById('batch-pause').click(); reader.uid = [0x5b,0x7d,0x40,0x3c]")
            wait("document.getElementById('number').textContent === '34935100'")
            assert js("return document.getElementById('batch-progress').textContent.includes('2 assigned')")
            js("document.getElementById('batch-skip').click()")
            wait("document.getElementById('batch-prompt').textContent.includes('Queue complete')")
            js("document.getElementById('batch-undo').click()")
            assert js("return document.getElementById('batch-prompt').textContent === 'Next: Absent Student'")
            js("document.getElementById('batch-skip').click(); document.getElementById('batch-download').click()")
            expected = [row.copy() for row in source_rows]
            expected[1][2], expected[2][2] = '34935097', '34935099'
            assert downloaded('students-cards.csv') == expected
            assert js("return dispatchEvent(new Event('beforeunload', {cancelable: true}))")
            # Invalid imports retain the previous batch.
            upload('bad.csv', 'Name,Card Number\nWrong,123\n')
            assert js("return document.getElementById('batch-error').textContent.includes('First name')")
            assert js("return document.getElementById('batch-progress').textContent.includes('2 assigned')")
            # Existing numbers are kept by default, and replacement is an explicit option.
            sample = (root / 'tests' / 'fixtures' / 'net2-import.csv').read_text(encoding='utf-8')
            upload('sample.csv', sample)
            assert js("return document.getElementById('batch-progress').textContent.includes('1 kept')")
            js("document.getElementById('batch-replace').checked = true; document.getElementById('batch-format').value = 'hex'")
            upload('hex.csv', sample)
            js("document.getElementById('batch-start').click()")
            time.sleep(.6)
            assert js("return document.getElementById('batch-progress').textContent.includes('0 assigned')")
            js('reader.card = false')
            wait("document.getElementById('number').textContent === '--------'")
            js('reader.card = true')
            wait("document.getElementById('batch-progress').textContent.includes('1 assigned')")
            js("document.getElementById('batch-download').click()")
            sample_rows = list(csv.reader(io.StringIO(sample)))
            sample_rows[1][3] = '5B7D403C'
            assert downloaded('hex-cards.csv') == sample_rows
            assert js('return errors') == []

            # Net2: sign in, upload a portrait, then save taps straight to Net2 users.
            js("window.confirm = () => true; location.hash = 'portraits'")
            wait("!document.getElementById('portrait-tool').hidden && !document.getElementById('net2-panel').hidden")
            assert js("return document.getElementById('reader-panel').hidden")

            def sign_in(password):
                fields = {'net2-server': 'https://NET2.test/', 'net2-client': 'client',
                          'net2-user': 'System engineer', 'net2-password': password + ' &=%'}
                js(f"""
                    for (const [id, value] of Object.entries({json.dumps(fields)}))
                        document.getElementById(id).value = value;
                    document.getElementById('net2-connect').click();
                """)
            sign_in('wrong')
            wait("document.getElementById('net2-message').textContent.includes('ClientID')")
            assert js("return document.getElementById('net2-password').value === ''")
            # A JSON sign-in needs a CORS preflight, which Net2's rate limit refuses.
            assert js("return net2.calls[0].kind") == 'application/x-www-form-urlencoded'
            sign_in('right')
            wait("document.getElementById('net2-message').textContent === 'Connected to https://net2.test'")
            assert js("return document.getElementById('net2-form').hidden")
            # A JPG goes up as it is. A PNG is converted: Net2 shows nothing for
            # one. A wide WebP is converted and shrunk; junk named like an image
            # is refused.
            js("""const canvas = document.createElement('canvas');
                  canvas.width = 3000; canvas.height = 600;
                  canvas.getContext('2d').fillRect(0, 0, 100, 100);
                  canvas.toBlob(blob => window.webp = blob, 'image/webp');
                  const small = document.createElement('canvas');
                  small.width = 10; small.height = 10;
                  small.toBlob(blob => window.png = blob, 'image/png');
                  small.toBlob(blob => blob.arrayBuffer().then(bytes => {
                      window.jpeg = blob;
                      window.jpegBase64 = btoa(String.fromCharCode(...new Uint8Array(bytes)));
                  }), 'image/jpeg');""")
            wait('window.webp && window.png && window.jpegBase64')
            js("""
                const transfer = new DataTransfer();
                transfer.items.add(new File([jpeg], '7.jpg'));
                for (const name of ['8.png', '007.png']) transfer.items.add(new File([png], name));
                transfer.items.add(new File([webp], '12.webp'));
                transfer.items.add(new File(['not an image'], '9.jpg'));
                const input = document.getElementById('portrait-files');
                input.files = transfer.files;
                input.dispatchEvent(new Event('change'));
            """)
            wait("document.querySelectorAll('#portrait-rows tr').length === 5 && !document.getElementById('portrait-review').disabled")
            assert js("return [...document.querySelectorAll('#portrait-rows tr')].map(r => !!r.querySelector('.problem'))") == [False, False, True, False, True]
            assert not js("return document.querySelector('#portrait-rows tr:nth-child(1) .converted')")
            assert js("return document.querySelector('#portrait-rows tr:nth-child(2) .converted').textContent") == 'Converted to JPG, 10×10'
            assert js("return document.querySelector('#portrait-rows tr:nth-child(4) .converted').textContent") == 'Converted to JPG, 1200×240'
            assert 'cannot open' in js("return document.querySelector('#portrait-rows tr:nth-child(5) .problem').textContent")
            js("document.getElementById('portrait-review').click()")
            wait("document.getElementById('portrait-notice').textContent.startsWith('Checked')")
            assert js("return document.querySelector('#portrait-rows tr td:nth-child(3)').textContent === 'Ada Lovelace'")
            assert js("return document.getElementById('portrait-upload').textContent === 'Upload 2 portraits'")
            js("document.getElementById('portrait-upload').click()")
            wait("document.getElementById('portrait-notice').textContent.startsWith('Done. 2 of 2')")
            assert js("return Object.keys(net2.images).join() === '7,12'")
            # Within Net2's limits the file goes up byte for byte; the WebP as a JPEG.
            assert js("return net2.images[7] === jpegBase64")
            assert js("return net2.images[12]").startswith('/9j/')

            js("""location.hash = 'batch';
                  const source = document.getElementById('batch-source');
                  source.value = 'net2';
                  source.dispatchEvent(new Event('change'));""")
            wait("!document.getElementById('batch-net2').hidden && document.querySelectorAll('#batch-department option').length === 2")
            assert js("return document.getElementById('batch-format').disabled && document.getElementById('batch-csv').hidden")
            assert js("return document.getElementById('batch-token-type').value") == 'ProxIsoCardWithoutMagstripe'
            js("""const types = document.getElementById('batch-token-type');
                  types.value = 'Keyfob';
                  types.dispatchEvent(new Event('change'));""")
            assert js("return localStorage.getItem('batch-token-type')") == 'Keyfob'

            js("""document.getElementById('batch-replace').checked = false;
                  document.getElementById('batch-department').value = '3';
                  document.getElementById('batch-load').click();""")
            wait("document.getElementById('batch-prompt').textContent === 'Next: Jane Doe'")
            js("document.getElementById('batch-start').click(); reader.card = false")
            wait("document.getElementById('number').textContent === '--------'")
            js('reader.card = true')
            wait("document.getElementById('batch-notice').textContent.includes('Jane Doe already has card 11111111')")
            assert js("return document.getElementById('batch-prompt').textContent.includes('John Roe')")
            js('reader.card = false')
            wait("document.getElementById('number').textContent === '--------'")
            js('reader.card = true')
            wait("document.getElementById('batch-progress').textContent.includes('1 assigned')")
            assert js("return JSON.stringify(net2.cards[12])") == '["34935100"]'
            posted = js("return net2.calls.filter(c => c.method === 'POST' && c.path.startsWith('/users/')).map(c => c.body)")
            assert posted == [{'tokenType': 'Keyfob', 'tokenValue': '34935100', 'isLost': False}]
            assert js("return document.getElementById('batch-prompt').textContent.includes('Queue complete')")
            assert js("return document.getElementById('batch-undo').disabled")
            assert js("return dispatchEvent(new Event('beforeunload', {cancelable: true}))")
            js("document.getElementById('batch-download').click()")
            assert downloaded('Net2 - Year 7-cards.csv') == [
                ['User ID', 'First name', 'Surname', 'Card Number'],
                ['8', 'Jane', 'Doe', ''],
                ['12', 'John', 'Roe', '34935100'],
            ]
            assert js("return net2.calls.every(c => c.path === '/authorization/tokens' || c.auth === 'Bearer T0K')")
            assert js('return errors') == []

            js('reader.unplugged = true; reader.opened = false')
            wait("document.getElementById('message').textContent.startsWith('Reader unplugged')")
            assert js('return errors') == []
            # A browser with no WebHID still loads the page and explains the limitation.
            command('goog/cdp/execute', {'cmd': 'Page.addScriptToEvaluateOnNewDocument',
                                       'params': {'source': 'delete navigator.hid; delete Navigator.prototype.hid;'}})
            command('refresh', {})
            wait("document.getElementById('message').textContent.startsWith('WebHID is not available')")
            assert js("return document.getElementById('connect').disabled")
            assert js('return errors') == []
            print('Static site smoke test passed: reader, batch upload, sequential taps, duplicate guard, pause, skip/undo, CSV preservation, decimal/hex downloads, Net2 sign-in, portrait upload and conversion, direct card saving, and no WebHID.')
        finally:
            try:
                if session:
                    request('DELETE', f'/session/{session}')
            finally:
                driver.terminate()
                driver.wait(timeout=10)
                server.shutdown()
                server.server_close()


if __name__ == '__main__':
    main()
