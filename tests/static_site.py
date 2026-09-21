"""Smoke-test the built dist directory in Chrome: python tests/static_site.py.

Uses only Python's standard library and an installed Chrome/chromedriver.
The fake HID and clipboard keep this test independent of hardware/permissions.
"""
import functools
import http.server
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
            print('Static site smoke test passed at /tusk/: connect, cancel, token, copy, theme, reload, removal, unplug, no WebHID.')
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
