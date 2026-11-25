"""Smoke-test the built dist directory in Chrome: python tests/static_site.py.

Uses only Python's standard library and an installed Chrome/chromedriver
(set CHROMEDRIVER to use another). A fake reader, clipboard and Net2 stand in
for the hardware and the server, so the test needs neither.

The scenarios at the bottom run in order in one page, each carrying on from
where the last one left it.
"""
import base64
import contextlib
import csv
import functools
import http.server
import io
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

# The reader, its permission prompt and the clipboard. `reader.card` puts a
# card on the reader or lifts it; `reader.uid` swaps in another card. Errors
# the page throws collect in `errors`. `$` is document.getElementById.
FAKE_READER = r"""
window.$ = id => document.getElementById(id);
window.errors = [];
addEventListener('error', e => errors.push(e.message));
addEventListener('unhandledrejection', e => errors.push(String(e.reason)));
window.reader = {
    vendorId: 0x1071, opened: false, card: true, unplugged: false, led: null, assignedSignals: 0,
    async open() { this.opened = true; },
    async sendReport(id, bytes) {
        if (this.unplugged) throw new DOMException('unplugged', 'NotFoundError');
        const key = new TextEncoder().encode('Elephant');
        const op = bytes[3] ^ ((bytes[2] & 128) ? key[bytes[2] % 8] : 0);
        if (op === 0x24) {
            const led = bytes[4] ^ ((bytes[2] & 128) ? key[(bytes[2] + 1) % 8] : 0);
            if (led === 6 && this.led !== 6) this.assignedSignals++;
            this.led = led;
        }
        if (op !== 0xd7 && op !== 0x14) return;
        const frame = uid => {
            const reply = new Uint8Array(37);
            reply.set([2, 37, 0, 16]);
            if (uid) reply.set(uid, 4);
            reply[36] = 255 - (reply.slice(0, 36).reduce((a, b) => a + b, 0) % 256);
            return reply;
        };
        // The Mifare frame captured in src/protocol.rs; an empty read is a valid ACK.
        let reply = new Uint8Array([
            2,37,165,113,53,9,5,85,101,112,104,97,110,116,69,108,
            101,112,104,97,110,116,69,108,101,112,104,97,110,116,69,108,
            101,112,104,97,249
        ]);
        if (op !== 0xd7 || !this.card) reply = frame();
        else if (this.uid) reply = frame(this.uid);
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

# A Net2 Local API at https://net2.test. Users 7 and 12 have no portrait or
# card, 8 has both, and 9's access has expired; people added through the API
# get IDs from 40. Every request is recorded in net2.calls. Set net2.token to
# anything else to expire the session.
FAKE_NET2 = r"""
window.net2 = {
    calls: [], queries: [], token: 'T0K',
    cards: {8: ['11111111']}, lost: [], images: {}, created: [],
    departments: [{id: 3, name: 'Year 7'}], edited: {}, permissions: {}, purged: [],
};
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
    const reply = (status, data) =>
        new Response(data === undefined ? null : JSON.stringify(data), {status});
    const route = (pattern, verb = method) => verb === method && path.match(pattern);

    // Signing in, and renewing with the refresh token.
    if (path === '/authorization/tokens') {
        if (body.grant_type === 'refresh_token') {
            if (body.refresh_token !== 'R3FRESH') return reply(400, {error: 'invalid_grant'});
            net2.token = 'T1K';
            return reply(200, {access_token: 'T1K', refresh_token: 'R3FRESH'});
        }
        return body.password === 'right &=%'
            ? reply(200, {access_token: net2.token, refresh_token: 'R3FRESH'})
            : reply(400, {message: 'invalid_client'});
    }
    if (auth !== 'Bearer ' + net2.token) return reply(401, {Message: 'Authorization has been denied'});
    if (method === 'PUT' && path === net2.failPath) return reply(400, {message: 'Test failure'});

    const users = {
        7: {id: 7, firstName: 'Ada', lastName: 'Lovelace', hasImage: false},
        8: {id: 8, firstName: 'Jane', lastName: 'Doe', hasImage: true, expiryDate: '0001-01-01T00:00:00'},
        9: {id: 9, firstName: 'Old', lastName: 'Leaver', hasImage: false, expiryDate: '2020-07-31T23:59:00'},
        12: {id: 12, firstName: 'John', lastName: 'Roe', hasImage: false},
        ...net2.edited,
    };
    for (const id of net2.purged) delete users[id];
    let m;

    // Lists.
    if (route(/^\/departments$/, 'GET')) return reply(200, net2.departments);
    if (route(/^\/departments$/, 'POST')) {
        net2.departments.push({id: 50 + net2.departments.filter(d => d.id >= 50).length, name: body.name});
        return reply(201, net2.departments.at(-1));
    }
    if ((m = route(/^\/departments\/(\d+)$/, 'PUT'))) {
        net2.departments.find(d => d.id === Number(m[1])).name = body.name;
        return reply(200, body);
    }
    if (path === '/departments/3/users') return reply(200, [users[8], users[9], users[12]]);
    if (path === '/accesslevels') return reply(200, [{id: 2, name: 'Car park'}, {id: 1, name: 'Working hours'}]);
    if (path === '/users/customfieldnames') return reply(200, [{id: 14, name: 'Admission number', maxLength: 50}]);
    if (path === '/events/latestunknowntokens?max=20') return reply(200, [
        {id: 90, tokenNumber: 78901234, eventDate: '2027-07-23T08:30:00', where: 'Reception'},
    ]);
    if (path.startsWith('/events?')) return reply(200, [
        {userID: 8, deviceName: 'Reception', eventTime: '2027-07-23T08:00:00', eventDescription: 'Access permitted'},
        {userID: 9, deviceName: 'Side entrance', eventTime: '2027-07-23T09:00:00', eventDescription: 'Wrong person'},
    ]);
    if (path === '/rollcallreports') return reply(200, [{id: 1, areaGroup: {name: 'Campus'}, dateTimeCreated: '2027-07-23T10:00:00'}]);
    if (path === '/rollcallreports/1/reportitems') return reply(200, [
        {userId: 8, firstName: 'Jane', lastName: 'Doe', lastKnownPosition: 'Library',
         lastAccessTime: '2027-07-23T08:00:00', musteredAt: 'Main gate', markedSafeBy: [{id: 1, name: 'Steward'}]},
    ]);
    if (path === '/doors') return reply(200, [{id: 42, name: 'Reception'}]);
    if (path.startsWith('/commands/door/')) return reply(200, {output: {
        status: path.endsWith('/open') ? 'CompletedFalied' : 'CompletedOk',
    }});

    // People: read, add, and their department, door permissions and portrait.
    if (route(/^\/users$/, 'GET')) return reply(200, Object.values(users).concat(net2.created));
    if (route(/^\/users$/, 'POST')) {
        const user = {...body, id: 40 + net2.created.length, hasImage: false};
        net2.created.push(user);
        if (net2.deferCreate) await new Promise(resolve => { net2.finishCreate = resolve; });
        if (net2.loseCreateReply) throw new TypeError('Lost creation reply');
        return reply(201, user);
    }
    if ((m = route(/^\/users\/(\d+)$/, 'GET'))) return users[m[1]] ? reply(200, users[m[1]]) : reply(404);
    if ((m = route(/^\/users\/(\d+)$/, 'PUT'))) {
        if (net2.persistManaged) net2.edited[m[1]] = body;
        return reply(200, body);
    }
    if ((m = route(/^\/users\/(\d+)\?purge=true$/, 'DELETE'))) {
        net2.purged.push(Number(m[1]));
        return reply(204);
    }
    if (route(/^\/users\/\d+\/departments$/, 'PUT')) return reply(204);
    if ((m = route(/^\/users\/(\d+)\/doorpermissionset$/))) {
        if (method === 'PUT') {
            // A real Net2 can answer 204 and keep less than it was sent.
            if (!net2.ignoreLevels) net2.permissions[m[1]] = body;
            return reply(204);
        }
        return reply(200, net2.permissions[m[1]]);
    }
    if ((m = route(/^\/users\/(\d+)\/image$/, 'PUT'))) {
        net2.images[m[1]] = body.base64Data;
        return reply(204);
    }

    // Cards: list, add, and mark lost.
    if ((m = route(/^\/users\/(\d+)\/tokens$/))) {
        const cards = net2.cards[m[1]] = net2.cards[m[1]] || [];
        if (method === 'POST') {
            if (net2.deferCard) await new Promise(resolve => { net2.finishCard = resolve; });
            cards.push(body.tokenValue);
            if (net2.loseCardReply) throw new TypeError('Lost card reply');
            return reply(201, body);
        }
        return reply(200, cards.map((tokenValue, i) =>
            ({id: i + 1, tokenType: 'ProxCard', tokenValue, isLost: net2.lost.includes(tokenValue)})));
    }
    if ((m = route(/^\/users\/(\d+)\/tokens\/(\d+)$/, 'PUT'))) {
        // As the real Net2 does: the body must name the token in the path.
        if (body.id !== Number(m[2])) {
            return reply(400, {message: 'Id of the token must match the token you are updating'});
        }
        if (body.isLost) net2.lost.push(net2.cards[m[1]][m[2] - 1]);
        return reply(200, body);
    }

    // Who holds a card, through the raw SQL endpoint.
    if ((m = route(/^\/customquery\/querydb\?query=(.*)$/, 'GET'))) {
        const sql = decodeURIComponent(m[1]);
        net2.queries.push(sql);
        if (sql.includes('[LastAccessTime]')) {
            return reply(200, [{lastaccesstime: '2027-07-23T08:00:00', lastarea: 'Library'}]);
        }
        const card = sql.match(/WHERE \[c\]\.\[CardNumber\] = (\d+) /)[1];
        return reply(200, Object.entries(net2.cards)
            .filter(([, cards]) => cards.includes(card))
            .map(([id]) => ({
                lost: net2.lost.includes(card), userid: +id, firstname: users[id].firstName,
                middlename: null, surname: users[id].lastName, department: 'Year 7',
            })));
    }
    return reply(404);
};
"""

FAKE_BROWSER = FAKE_READER + FAKE_NET2

# What the number shows with no card on the reader.
BLANK = '--------'


def csv_text(rows: list[list[str]]) -> str:
    text = io.StringIO(newline='')
    csv.writer(text).writerows(rows)
    return text.getvalue()


def csv_rows(text: str) -> list[list[str]]:
    return list(csv.reader(io.StringIO(text)))


def free_port() -> int:
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        return sock.getsockname()[1]


class Browser:
    """One Chrome session, driven over WebDriver."""

    def __init__(self, port: int, downloads: Path):
        self.port = port
        self.downloads = downloads
        self.session = None

    def request(self, method: str, path: str, data: dict | None = None):
        body = None if data is None else json.dumps(data).encode()
        req = urllib.request.Request(f'http://127.0.0.1:{self.port}{path}', data=body,
                                     method=method, headers={'Content-Type': 'application/json'})
        with urllib.request.urlopen(req, timeout=20) as response:
            return json.load(response)['value']

    def command(self, path: str, data: dict):
        return self.request('POST', f'/session/{self.session}/{path}', data)

    def cdp(self, cmd: str, params: dict | None = None):
        return self.command('goog/cdp/execute', {'cmd': cmd, 'params': params or {}})

    def js(self, script: str):
        return self.command('execute/sync', {'script': script, 'args': []})

    def wait(self, expression: str, timeout: float = 8):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if self.js('return ' + expression):
                return
            time.sleep(.05)
        raise AssertionError(f'Timed out: {expression}; errors: {self.js("return window.errors")}')

    # Elements by id.

    def text(self, id: str) -> str:
        return self.js(f"return $('{id}').textContent")

    def click(self, *ids: str):
        for id in ids:
            self.js(f"$('{id}').click()")

    def shown(self, id: str):
        """Wait for a tool, panel or row to appear."""
        self.wait(f"!$('{id}').hidden")

    def wait_text(self, id: str, expected: str):
        self.wait(f"$('{id}').textContent === {json.dumps(expected)}")

    def wait_contains(self, id: str, part: str):
        self.wait(f"$('{id}').textContent.includes({json.dumps(part)})")

    def wait_starts(self, id: str, start: str):
        self.wait(f"$('{id}').textContent.startsWith({json.dumps(start)})")

    # The reader.

    def tap(self, uid: str | None = None):
        """Put a card on the reader: the captured one, or one with this UID."""
        uid = f'reader.uid = {list(bytes.fromhex(uid))};' if uid else ''
        self.js(f'{uid} reader.card = true')

    def lift(self):
        self.js('reader.card = false')
        self.wait_text('number', BLANK)

    # Files in and out.

    def upload(self, field: str, name: str, content: str):
        """Choose a text file in a file picker, as the operator would."""
        self.js(f"""
            const transfer = new DataTransfer();
            transfer.items.add(new File([{json.dumps(content)}], {json.dumps(name)}, {{type: 'text/csv'}}));
            const input = $('{field}');
            input.files = transfer.files;
            input.dispatchEvent(new Event('change', {{bubbles: true}}));
        """)
        self.wait(f"!$('{field}').disabled")

    def choose_files(self, field: str, files: str):
        """Choose files built in the page. `files` is JS: a list of [blob, name]."""
        self.js(f"""
            const transfer = new DataTransfer();
            for (const [blob, name] of {files}) transfer.items.add(new File([blob], name));
            const input = $('{field}');
            input.files = transfer.files;
            input.dispatchEvent(new Event('change'));
        """)

    def downloaded(self, name: str) -> list[list[str]]:
        path = self.downloads / name
        deadline = time.monotonic() + 8
        while time.monotonic() < deadline:
            if path.exists():
                return csv_rows(path.read_text(encoding='utf-8-sig'))
            time.sleep(.05)
        raise AssertionError(f'Download missing: {path}')

    def fits_desktop_and_phone(self, name: str, height: int, scroll_to: str | None = None):
        """No sideways scrolling at either width. Keeps screenshots in target/."""
        for width, label in [(1100, 'desktop'), (390, 'mobile')]:
            self.cdp('Emulation.setDeviceMetricsOverride', {
                'width': width, 'height': height, 'deviceScaleFactor': 1, 'mobile': False,
            })
            if scroll_to:
                self.js(f"$('{scroll_to}').scrollIntoView()")
            assert self.js('return document.documentElement.scrollWidth <= innerWidth'), (name, label)
            png = base64.b64decode(self.request('GET', f'/session/{self.session}/screenshot'))
            (ROOT / 'target' / f'{name}-{label}.png').write_bytes(png)
        self.cdp('Emulation.clearDeviceMetricsOverride')


class QuietHandler(http.server.SimpleHTTPRequestHandler):
    """Serves the site without a log line per request."""

    def log_message(self, *args):
        pass


class QuietServer(http.server.ThreadingHTTPServer):
    """Chrome drops connections when the test reloads the page; that is not
    a failure worth a traceback."""

    def handle_error(self, request, client_address):
        if not isinstance(sys.exc_info()[1], ConnectionError):
            super().handle_error(request, client_address)


@contextlib.contextmanager
def serve(dist: Path):
    """The built site under /tusk/, as GitHub Pages serves it, and a
    scratch directory beside it."""
    with tempfile.TemporaryDirectory() as directory:
        shutil.copytree(dist, Path(directory) / 'tusk')
        handler = functools.partial(QuietHandler, directory=directory)
        server = QuietServer(('127.0.0.1', 0), handler)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        try:
            yield f'http://127.0.0.1:{server.server_port}/tusk/', Path(directory)
        finally:
            server.shutdown()
            server.server_close()


@contextlib.contextmanager
def chrome(downloads: Path):
    """Headless Chrome with the fakes in every page, saving downloads to
    `downloads`."""
    port = free_port()
    driver = subprocess.Popen(
        [os.environ.get('CHROMEDRIVER', 'chromedriver'), f'--port={port}'],
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        creationflags=getattr(subprocess, 'CREATE_NO_WINDOW', 0),
    )
    browser = Browser(port, downloads)
    try:
        for _ in range(100):
            try:
                browser.request('GET', '/status')
                break
            except OSError:
                if driver.poll() is not None:
                    raise RuntimeError('chromedriver exited before starting')
                time.sleep(.05)
        browser.session = browser.request('POST', '/session', {'capabilities': {'alwaysMatch': {
            'browserName': 'chrome',
            'goog:chromeOptions': {'args': ['--headless=new', '--no-sandbox', '--disable-dev-shm-usage']},
        }}})['sessionId']
        browser.cdp('Page.addScriptToEvaluateOnNewDocument', {'source': FAKE_BROWSER})
        browser.cdp('Browser.setDownloadBehavior', {'behavior': 'allow', 'downloadPath': str(downloads)})
        yield browser
    finally:
        try:
            if browser.session:
                browser.request('DELETE', f'/session/{browser.session}')
        finally:
            driver.terminate()
            driver.wait(timeout=10)


# The scenarios, in the order they run.

def reading_and_copying_a_card(b: Browser):
    b.wait_text('message', 'Not connected')
    assert b.js("return $('copy-number').disabled")
    b.click('connect')
    b.wait_text('message', 'No reader selected')
    b.js('cancelPicker = false')
    b.click('connect')
    b.wait_text('number', '34935097')
    assert b.text('hex') == '5B7D4039'
    for control, expected in [('copy-number', '34935097'), ('copy-hex', '5B7D4039')]:
        b.click(control)
        b.wait(f"window.copied === '{expected}' && $('{control}').dataset.copied === 'true'")
    # The theme survives a reload, and the reader reconnects by itself.
    b.click('theme')
    theme = b.js('return document.documentElement.dataset.theme')
    b.command('refresh', {})
    b.wait_text('number', '34935097')
    assert b.js('return document.documentElement.dataset.theme') == theme
    b.lift()
    assert b.js("return $('copy-number').disabled && $('hex').hidden")


def batch_assign_from_a_spreadsheet(b: Browser):
    # The real file picker handler and download, in the built site.
    b.js("location.hash = 'batch'")
    b.shown('batch-tool')
    source_rows = [
        ['Surname', 'First name', 'Card Number', 'Notes', 'User ID'],
        ['Doe', 'John', '', 'Comma, quote " and\nnewline', '001'],
        ['Dawkins', 'Jane', '', '', '002', ''],
        ['<img src=x onerror=alert(1)>', 'Éva', '00012345', 'keep', '003'],
        ['Student', 'Absent', '', 'skip', '004'],
    ]
    b.upload('batch-file', 'students.csv', csv_text(source_rows))
    assert b.js("return $('batch-format').value") == 'decimal'
    assert b.text('batch-prompt') == 'Next: John Doe'
    assert b.js("return document.querySelectorAll('#batch-rows img').length") == 0

    b.click('batch-start')
    b.tap()
    b.wait_contains('batch-progress', '1 assigned')
    b.wait('reader.assignedSignals === 1')
    assert b.js("return !dispatchEvent(new Event('beforeunload', {cancelable: true}))"), 'unsaved work warns'
    time.sleep(.8)  # A held card must not go to the next people too.
    assert 'Jane Dawkins' in b.text('batch-prompt')
    # Once, as Net2 does: repeated, the real reader stayed lit for minutes.
    assert b.js('return reader.assignedSignals') == 1 and b.js('return reader.led') == 10
    b.lift()
    b.wait('reader.led === 10')
    signals = b.js('return reader.assignedSignals')
    b.tap()
    b.wait_contains('batch-notice', 'already belongs to John Doe')
    assert b.js('return reader.led') == 10
    assert b.js('return reader.assignedSignals') == signals
    b.tap('5B7D403B')
    b.wait_contains('batch-progress', '2 assigned')
    b.wait(f'reader.assignedSignals === {signals + 1}')
    b.fits_desktop_and_phone('batch', 900, scroll_to='batch-session')

    # Paused, a tap changes nothing; skip, undo and skip again, then download.
    b.click('batch-pause')
    b.tap('5B7D403C')
    b.wait_text('number', '34935100')
    assert '2 assigned' in b.text('batch-progress')
    b.click('batch-skip')
    b.wait_contains('batch-prompt', 'Queue complete')
    b.click('batch-undo')
    b.wait('reader.led === 10')
    assert b.text('batch-prompt') == 'Next: Absent Student'
    b.click('batch-skip', 'batch-download')
    expected = [row.copy() for row in source_rows]
    expected[1][2], expected[2][2] = '34935097', '34935099'
    assert b.downloaded('students-cards.csv') == expected
    assert b.js("return dispatchEvent(new Event('beforeunload', {cancelable: true}))"), 'saved work does not warn'

    # A file that cannot be a queue keeps the one already loaded.
    b.upload('batch-file', 'bad.csv', 'Name,Card Number\nWrong,123\n')
    assert 'First name' in b.text('batch-error')
    assert '2 assigned' in b.text('batch-progress')

    # Other headings for the same columns are matched; a column no heading
    # names is picked by hand, and the queue is rebuilt from it.
    b.upload('batch-file', 'tabs.tsv', 'Forename\tFAMILY_NAME\tTag\nAda\tLovelace\t\n')
    assert b.text('batch-error') == 'Pick the Card Number column.'
    assert b.js("return $('batch-columns').open")
    b.js("""const card = document.querySelectorAll('#batch-map select')[2];
            card.value = '2';
            card.dispatchEvent(new Event('change', {bubbles: true}));""")
    b.wait_text('batch-prompt', 'Next: Ada Lovelace')

    # Existing numbers are kept by default; replacing them is asked for.
    sample = (ROOT / 'tests' / 'fixtures' / 'net2-import.csv').read_text(encoding='utf-8')
    b.upload('batch-file', 'sample.csv', sample)
    assert '1 kept' in b.text('batch-progress')
    b.js("$('batch-replace').checked = true; $('batch-format').value = 'hex'")
    b.upload('batch-file', 'hex.csv', sample)
    b.click('batch-start')
    time.sleep(.6)  # The card already on the reader at the start is not taken.
    assert '0 assigned' in b.text('batch-progress')
    b.lift()
    b.tap()
    b.wait_contains('batch-progress', '1 assigned')
    b.click('batch-download')
    sample_rows = csv_rows(sample)
    sample_rows[1][3] = '5B7D403C'
    assert b.downloaded('hex-cards.csv') == sample_rows


SHEET_OPEN = "$('net2-panel').matches(':popover-open')"


def signing_in_to_net2(b: Browser):
    # The sign-in sheet opens from any tab; on Read a card the holder pane
    # offers it too.
    b.js("location.hash = 'reader'")
    b.shown('reading')
    assert b.js(f"return !{SHEET_OPEN} && $('owner').hidden")
    b.click('owner-connect')
    b.wait(SHEET_OPEN)
    b.js("$('net2-panel').hidePopover()")

    b.js("window.confirm = () => true; location.hash = 'portraits'")
    b.shown('portrait-tool')
    # The reader and Net2 links stay in view on every tab.
    assert b.js("return !$('reader-panel').hidden && !$('net2-open').hidden")

    def sign_in(password: str):
        fields = {'net2-server': 'https://NET2.test/', 'net2-client': 'client',
                  'net2-user': 'System engineer', 'net2-password': password + ' &=%'}
        b.js(f"""
            if (!{SHEET_OPEN}) $('net2-open').click();
            for (const [id, value] of Object.entries({json.dumps(fields)})) $(id).value = value;
            $('net2-connect').click();
        """)

    sign_in('wrong')
    b.wait_contains('net2-message', 'ClientID')
    # The sheet covers the status bar, so it repeats the refusal and stays open.
    assert b.js(f"return {SHEET_OPEN}")
    assert 'ClientID' in b.text('net2-problem')
    assert b.js("return $('net2-password').value") == ''
    # A JSON sign-in needs a CORS preflight, which Net2's rate limit refuses.
    assert b.js('return net2.calls[0].kind') == 'application/x-www-form-urlencoded'

    sign_in('right')
    b.wait_text('net2-message', 'Connected to https://net2.test')
    assert b.js(f"return $('net2-form').hidden && !{SHEET_OPEN}")
    assert b.js("return $('net2-open').hidden && !$('net2-disconnect').hidden")


def uploading_portraits(b: Browser):
    # A JPG goes up as it is. A PNG is converted: Net2 shows nothing for one.
    # A wide WebP is converted and shrunk; junk named like an image is refused.
    b.js("""const wide = document.createElement('canvas');
            wide.width = 3000; wide.height = 600;
            wide.getContext('2d').fillRect(0, 0, 100, 100);
            wide.toBlob(blob => window.webp = blob, 'image/webp');
            const small = document.createElement('canvas');
            small.width = 10; small.height = 10;
            small.toBlob(blob => window.png = blob, 'image/png');
            small.toBlob(blob => blob.arrayBuffer().then(bytes => {
                window.jpeg = blob;
                window.jpegBase64 = btoa(String.fromCharCode(...new Uint8Array(bytes)));
            }), 'image/jpeg');""")
    b.wait('window.webp && window.png && window.jpegBase64')
    b.choose_files('portrait-files', """[
        [jpeg, '7.jpg'], [png, '8.png'], [png, '007.png'], [webp, '12.webp'], ['not an image', '9.jpg'],
    ]""")
    b.wait("document.querySelectorAll('#portrait-rows tr').length === 5 && !$('portrait-review').disabled")
    problems = "[...document.querySelectorAll('#portrait-rows tr')].map(row => !!row.querySelector('.problem'))"
    assert b.js(f'return {problems}') == [False, False, True, False, True]
    note = "document.querySelector('#portrait-rows tr:nth-child({}) .{}')"
    assert not b.js(f"return {note.format(1, 'converted')}")
    assert b.js(f"return {note.format(2, 'converted')}.textContent") == 'Converted to JPG, 10×10'
    assert b.js(f"return {note.format(4, 'converted')}.textContent") == 'Converted to JPG, 1200×240'
    assert 'cannot open' in b.js(f"return {note.format(5, 'problem')}.textContent")

    b.click('portrait-review')
    b.wait_starts('portrait-notice', 'Checked')
    assert b.js("return document.querySelector('#portrait-rows tr td:nth-child(3)').textContent") == 'Ada Lovelace'
    assert b.text('portrait-upload') == 'Upload 2 portraits'
    b.click('portrait-upload')
    b.wait_starts('portrait-notice', 'Done. 2 of 2')
    assert b.js('return Object.keys(net2.images)') == ['7', '12']
    # Within Net2's limits the file goes up byte for byte; the WebP as a JPEG.
    assert b.js('return net2.images[7] === jpegBase64')
    assert b.js('return net2.images[12]').startswith('/9j/')


# Each row's result on Add people, and whether it offers Add anyway.
RESULTS = "[...document.querySelectorAll('#people-rows td:last-child > span')].map(span => span.textContent)"
OFFERED = "[...document.querySelectorAll('#people-rows .anyway')].map(label => !label.hidden)"


def warns_before_leaving(b: Browser) -> bool:
    return b.js("""
        const event = new Event('beforeunload', {cancelable: true});
        window.dispatchEvent(event);
        return event.defaultPrevented;
    """)


def adding_people(b: Browser):
    b.js("location.hash = 'people'")
    b.shown('people-tool')
    assert b.js("return $('people-signin').hidden"), 'signed in already'
    b.upload('people-file', 'people.csv', csv_text([
        ['Forename', 'Surname', 'Dept', 'Access level', 'Expiry', 'Admission number', 'Photo', 'Active'],
        ['Riya', 'Patel', 'year 7', 'working hours', '31/07/2027', 'A1', r'C:\photos\Riya.PNG', '16/10/2026'],
        ['Ada', 'Lovelace', '', '', '', '', '', ''],
        ['Tom', 'Hall', 'Year 9', '', '', '', '', ''],
        ['Sam', 'Lee', '', '', '', '', 'sam.jpg', ''],
        ['Kim', 'Wu', '', '', '07/31/2027', '', '', ''],
        ['riya', 'PATEL', '', '', '', '', '', ''],
        ['Zoe', 'King', '', 'Night shift', '', '', '', ''],
    ]))
    b.wait("document.querySelectorAll('#people-rows tr').length === 7")
    assert b.js("return $('people-results').disabled")
    assert not warns_before_leaving(b)
    # Net2's own name for custom field 14 is what the column matched.
    pickers = "document.querySelectorAll('#people-map select')"
    assert b.js("return [...document.querySelectorAll('#people-map label')].at(-1).textContent").startswith('Admission number')
    assert b.js(f'return {pickers}[13].value') == '5'
    assert b.js(f'return {pickers}[1].selectedOptions[0].textContent') == 'Not used'
    assert b.js(f'return {pickers}[5].value') == ''
    b.js(f"{pickers}[5].value = '7'; {pickers}[5].dispatchEvent(new Event('change', {{bubbles: true}}))")
    preview = "[...document.querySelector('#people-rows tr').children].map(td => td.textContent)"
    assert b.js(f'return {preview}')[3:6] == ['working hours', '16/10/2026', '31/07/2027']

    b.choose_files('people-photos', "[[png, 'riya.png']]")
    b.click('people-check')
    b.wait_starts('people-notice', 'Checked: 1 ready, 6 held back')
    assert b.js(f'return {preview}')[3:6] == ['Working hours', '2026-10-16', '2027-07-31']
    assert b.js("return [...document.querySelector('#people-rows tr:nth-child(2)').children].slice(3, 6).map(td => td.textContent)") == ['', '', '']
    assert b.js("return document.querySelector('#people-rows tr:nth-child(5) td:nth-child(6)').textContent") == '07/31/2027'
    results = b.js(f'return {RESULTS}')
    assert results[4].startswith('Expiration date:'), results
    assert results == [
        'Ready',
        'Same name as user 7 in Net2.',
        'Net2 has no department Year 9.',
        'No photo called sam.jpg was chosen.',
        results[4],
        'Same name as row 2.',
        'Net2 has no access level Night shift.',
    ], results
    # Only a name match can be added anyway.
    assert b.js(f'return {OFFERED}') == [False, True, False, False, False, True, False]
    b.fits_desktop_and_phone('people', 1400)

    # The Ada Lovelace in the sheet is someone else: add her anyway.
    b.js("document.querySelector('#people-rows tr:nth-child(2) .anyway input').click()")
    b.wait_text('people-create', 'Add 2 people to Net2')
    b.js('window.asked = []; window.confirm = question => (asked.push(question), true)')
    b.click('people-create')
    b.wait_starts('people-notice', 'Done. 2 of 2 added')
    assert '1 of them share a name' in b.js('return asked[0]'), b.js('return asked')
    b.js('window.confirm = () => true')

    assert b.js('return net2.created.map(user => user.id)') == [40, 41]
    assert b.js('return net2.created[0]') == {
        'firstName': 'Riya', 'middleName': '', 'lastName': 'Patel', 'isAlarmUser': False,
        'expiryDate': '2027-07-31T23:59:00', 'customFields': [{'id': 14, 'value': 'A1'}],
        'activateDate': '2026-10-16T00:00:00',
        'id': 40, 'hasImage': False,
    }
    # Then her department, access level and portrait, in that order.
    writes = b.js(r"return net2.calls.filter(c => /^\/users\/4\d\//.test(c.path)).map(c => [c.method, c.path, c.body])")
    assert [w[:2] for w in writes] == [
        ['PUT', '/users/40/departments'], ['PUT', '/users/40/doorpermissionset'], ['PUT', '/users/40/image'],
    ], writes
    assert writes[0][2] == {'id': 3, 'name': 'Year 7'}, writes
    assert writes[1][2] == {'accessLevels': [1], 'individualPermissions': []}, writes
    assert b.js('return net2.images[40]').startswith('/9j/')
    assert b.js(f'return {RESULTS}')[:2] == ['Added as user 40.', 'Added as user 41.']

    b.click('people-download')
    people_back = b.downloaded('people-net2.csv')
    assert [row[-1] for row in people_back[:4]] == ['User ID', '40', '41', ''], people_back
    assert warns_before_leaving(b), 'the reimport CSV omits outcome messages'
    b.click('people-results')
    assert b.downloaded('people-results.csv') == [
        ['Sheet row', 'Name', 'User ID', 'Result'],
        ['2', 'Riya Patel', '40', 'Added as user 40.'],
        ['3', 'Ada Lovelace', '41', 'Added as user 41.'],
    ]
    assert not warns_before_leaving(b)

    # Run again from the download: nobody is added twice, and the repeated
    # Riya now matches the one just added.
    b.upload('people-file', 'people-net2.csv', csv_text(people_back))
    b.wait("document.querySelectorAll('#people-rows tr').length === 7")
    b.click('people-check')
    b.wait_starts('people-notice', 'Checked: 0 ready')
    results = b.js(f'return {RESULTS}')
    assert results[:2] == ['Already added: user 40.', 'Already added: user 41.'], results
    assert results[5] == 'Same name as user 40 in Net2.', results
    assert b.js(f'return {OFFERED}')[:2] == [False, False]
    assert b.js("return $('people-create').disabled")


def keeping_people_import_results(b: Browser):
    sheet = [['First name', 'Surname', 'Department', 'Access level', 'PIN'],
             ['Mary', 'Hall', 'Year 7', 'Working hours', '9876']]
    b.upload('people-file', 'partial.csv', csv_text(sheet))
    b.click('people-check')
    b.wait_starts('people-notice', 'Checked: 1 ready')
    b.js("net2.failPath = '/users/42/doorpermissionset'; net2.deferCreate = true")
    b.click('people-create')
    b.wait('!!net2.finishCreate')
    assert warns_before_leaving(b), 'protect the import before Net2 replies'
    assert b.js("return $('people-file').disabled && $('people-create').disabled && $('people-results').disabled")
    b.js('delete net2.deferCreate; net2.finishCreate(); delete net2.finishCreate')
    b.wait_starts('people-notice', 'Done. 1 of 1 added')
    result = b.js(f'return {RESULTS}[0]')
    assert 'No access levels:' in result and 'Test failure' in result, result
    assert warns_before_leaving(b)
    assert '9876' not in b.text('people-rows'), 'PINs are not previewed'

    b.js('window.asked = []; window.confirm = question => (asked.push(question), false)')
    b.upload('people-file', 'replacement.csv', csv_text(sheet))
    b.wait_starts('people-notice', 'Kept the current sheet')
    assert len(b.js('return asked')) == 1
    assert b.js(f'return {RESULTS}[0]') == result
    assert warns_before_leaving(b)
    # Checking again must not overwrite the recorded creation outcome.
    b.click('people-check')
    b.wait_starts('people-notice', 'Checked: 0 ready')
    b.click('people-results')
    assert b.downloaded('partial-results.csv') == [
        ['Sheet row', 'Name', 'User ID', 'Result'], ['2', 'Mary Hall', '42', result],
    ], 'report contains outcomes, without PINs or other source columns'
    assert not warns_before_leaving(b)
    b.fits_desktop_and_phone('people-results', 1400)

    # Net2 creates the user but its response is lost: no ID, no automatic retry.
    b.js('window.confirm = () => true; delete net2.failPath; net2.loseCreateReply = true')
    b.upload('people-file', 'uncertain.csv', csv_text([
        ['First name', 'Surname'], ['Eve', 'Young'], ['Not', 'Attempted'],
    ]))
    b.click('people-check')
    b.wait_starts('people-notice', 'Checked: 2 ready')
    b.click('people-create')
    b.wait_starts('people-notice', 'Stopped. 0 of 2 added')
    assert b.js('return net2.created.length') == 4
    assert b.js("return $('people-download').disabled && !$('people-results').disabled")
    assert warns_before_leaving(b)
    uncertain = b.js(f'return {RESULTS}[0]')
    b.click('people-results')
    assert b.downloaded('uncertain-results.csv') == [
        ['Sheet row', 'Name', 'User ID', 'Result'], ['2', 'Eve Young', '', uncertain],
    ]
    assert not warns_before_leaving(b)

    # A failed download must leave the warning in place; accepting replacement clears it.
    b.js('delete net2.loseCreateReply')
    b.click('people-create')
    b.wait_starts('people-notice', 'Done. 1 of 1 added')
    b.js("window.realObjectURL = URL.createObjectURL; URL.createObjectURL = () => { throw Error('Download failed'); }")
    b.click('people-results')
    b.wait_text('people-notice', 'Could not create a download link.')
    assert warns_before_leaving(b)
    b.js('URL.createObjectURL = realObjectURL; window.asked = []; window.confirm = question => (asked.push(question), true)')
    b.upload('people-file', 'replacement.csv', csv_text(sheet))
    assert len(b.js('return asked')) == 1
    assert not warns_before_leaving(b)
    assert b.js("return $('people-results').disabled")


def saving_cards_straight_to_net2(b: Browser):
    b.js("""location.hash = 'batch';
            $('batch-source').value = 'net2';
            $('batch-source').dispatchEvent(new Event('change'));""")
    b.wait("!$('batch-net2').hidden && document.querySelectorAll('#batch-department option').length === 2")
    assert b.js("return $('batch-format').disabled && $('batch-csv').hidden")
    assert b.js("return $('batch-token-type').value") == 'ProxIsoCardWithoutMagstripe'
    b.js("$('batch-token-type').value = 'Keyfob'; $('batch-token-type').dispatchEvent(new Event('change'))")
    assert b.js("return localStorage.getItem('batch-token-type')") == 'Keyfob'

    # A department made in Net2 after signing in appears when the page
    # regains focus, and the one already chosen stays chosen.
    b.js("""$('batch-department').value = '3';
            net2.departments.push({id: 4, name: 'Tusk test'});
            dispatchEvent(new Event('focus'));""")
    b.wait("document.querySelectorAll('#batch-department option').length === 3")
    assert b.js("return $('batch-department').value") == '3'

    # Net2 expires the session; loading renews it once and carries on.
    b.js("""net2.token = 'EXPIRED';
            $('batch-replace').checked = false;
            $('batch-department').value = '3';""")
    b.click('batch-load')
    b.wait_text('batch-prompt', 'Next: Jane Doe')
    assert b.js('return net2.calls.filter(c => c.body && c.body.grant_type === "refresh_token").length') == 1
    assert b.js('return net2.calls.find(c => c.body && c.body.password === "right &=%").body.scope') == 'offline_access'
    assert b.text('net2-message') == 'Connected to https://net2.test'

    # Expired people are listed but never asked for.
    assert b.text('batch-notice').endswith('1 person whose Net2 access has expired.')
    assert b.text('batch-progress').endswith('· 1 expired')
    assert b.js("return document.querySelector('#batch-rows tr:nth-child(2) td:last-child').textContent") == 'Expired'

    # Jane already has a card, so she keeps it; John gets the tap.
    b.click('batch-start')
    b.lift()
    b.tap()
    b.wait_contains('batch-notice', 'Jane Doe already has card 11111111')
    b.wait('reader.led === 10')
    assert 'John Roe' in b.text('batch-prompt')
    b.lift()
    b.js('net2.deferCard = true')
    b.tap()
    b.wait('!!net2.finishCard')
    assert b.js('return reader.led') == 10, 'pending saves do not signal success'
    signals = b.js('return reader.assignedSignals')
    b.js('delete net2.deferCard; net2.finishCard(); delete net2.finishCard')
    b.wait_contains('batch-progress', '1 assigned')
    b.wait(f'reader.assignedSignals === {signals + 1}')
    assert b.js('return net2.cards[12]') == ['34935100']
    posted = b.js("return net2.calls.filter(c => c.method === 'POST' && c.path.startsWith('/users/')).map(c => c.body)")
    assert posted == [{'tokenType': 'Keyfob', 'tokenValue': '34935100', 'isLost': False}]
    assert 'Queue complete' in b.text('batch-prompt')
    # A card saved to Net2 is not undone here, and nothing is left unsaved.
    assert b.js("return $('batch-undo').disabled")
    assert b.js("return dispatchEvent(new Event('beforeunload', {cancelable: true}))")
    b.click('batch-download')
    assert b.downloaded('Net2 - Year 7-cards.csv') == [
        ['User ID', 'First name', 'Surname', 'Card Number'],
        ['8', 'Jane', 'Doe', ''],
        ['9', 'Old', 'Leaver', ''],
        ['12', 'John', 'Roe', '34935100'],
    ]


def reissuing_marks_old_cards_lost(b: Browser):
    # Jane's new card goes in first, then her old one is marked lost, and
    # only that one.
    b.js("$('batch-replace').checked = true; $('batch-retire').checked = true")
    b.click('batch-load')
    b.wait_text('batch-prompt', 'Next: Jane Doe')
    b.click('batch-start')
    b.lift()
    b.tap('5B7D4050')
    b.wait_contains('batch-notice', 'Old card 11111111 is marked lost.')
    assert b.js('return net2.cards[8].length') == 2
    assert b.js('return net2.lost') == ['11111111']
    writes = b.js("return net2.calls.filter(c => c.method !== 'GET' && c.path.startsWith('/users/8/')).map(c => [c.method, c.path, c.body])")
    assert [w[:2] for w in writes] == [['POST', '/users/8/tokens'], ['PUT', '/users/8/tokens/1']], writes
    assert writes[1][2] == {'id': 1, 'tokenType': 'ProxCard', 'tokenValue': '11111111', 'isLost': True}, writes
    assert 'John Roe' in b.text('batch-prompt')


def looking_up_who_holds_a_card(b: Browser):
    # Each tap on Read a card is looked up once.
    b.js("location.hash = 'reader'")
    b.lift()
    b.wait("$('owner').hidden")
    asked = b.js('return net2.queries.length')
    b.tap('5B7D403C')
    b.wait_text('owner', 'John Roe (user 12) · Year 7')
    assert b.js('return net2.queries.length') == asked + 1
    assert b.js('return net2.queries.at(-1)').endswith('WHERE [c].[CardNumber] = 34935100 ORDER BY [c].[LostCard] ASC')
    b.js('reader.card = false')
    b.wait("$('owner').hidden")
    b.tap('01020304')
    b.wait_text('owner', 'Not in Net2')
    assert b.js('return net2.queries.length') == asked + 2
    # Every request but signing in carried the session's token.
    assert b.js("return net2.calls.every(c => c.path === '/authorization/tokens' || c.auth === 'Bearer T0K' || c.auth === 'Bearer T1K')")


def reader_feedback_waits_for_the_saved_card(b: Browser):
    b.js("location.hash = 'batch'; $('batch-retire').checked = false")
    b.shown('batch-tool')
    b.click('batch-load')
    b.wait_text('batch-prompt', 'Next: Jane Doe')
    b.click('batch-start')
    b.lift()
    b.wait('reader.led === 10')
    signals = b.js('return reader.assignedSignals')
    b.js('net2.deferCard = true')
    b.tap('5B7D4060')
    b.wait('!!net2.finishCard')
    b.lift()
    b.js('delete net2.deferCard; net2.finishCard(); delete net2.finishCard')
    b.wait_contains('batch-progress', '1 assigned')
    time.sleep(.4)
    assert b.js('return reader.led') == 10, 'a late save must not light an empty reader'
    assert b.js('return reader.assignedSignals') == signals

    b.js('net2.loseCardReply = true')
    b.tap('5B7D4061')
    b.wait_contains('batch-notice', 'Paused.')
    time.sleep(.4)
    assert b.js('return reader.led') == 10, 'uncertain writes do not signal success'
    assert b.js('return reader.assignedSignals') == signals
    assert b.js("return $('batch-start').disabled"), 'uncertain outcomes need reconciliation'
    b.js('delete net2.loseCardReply')


MANAGED = "[...document.querySelectorAll('#manage-rows tr')].map(tr => [...tr.children].slice(1).map(td => td.textContent))"


def managing_people(b: Browser):
    # Net2 gets more departments; the page reads them again when it regains focus.
    b.js("net2.departments.push({id: 10, name: 'Year 8'}, {id: 11, name: 'Leavers'}, {id: 12, name: 'Staff'})")
    b.js("location.hash = 'manage'")
    b.shown('manage-tool')
    assert b.js("return $('manage-signin').hidden"), 'signed in already'
    b.js("dispatchEvent(new Event('focus'))")
    b.wait("$('manage-move-to').options.length === 6")
    assert b.js("return [...$('manage-leavers-to').options].map(o => o.textContent)") == [
        'The department they are in', 'Leavers', 'Staff', 'Tusk test', 'Year 7', 'Year 8',
    ]
    b.js("$('manage-department').value = '3'")
    b.click('manage-load')
    b.wait_starts('manage-notice', '3 people in Year 7')
    # By surname; a placeholder expiry is no expiry.
    assert b.js(f'return {MANAGED}') == [
        ['Jane Doe', '8', '', ''],
        ['Old Leaver', '9', '2020-07-31 (ended)', ''],
        ['John Roe', '12', '', ''],
    ]
    assert b.js("return $('manage-move').disabled && $('manage-leave').disabled")
    b.click('manage-all')
    b.wait_text('manage-move', 'Move 3 people')
    b.js("document.querySelector('#manage-rows tr:nth-child(2) input').click()")
    b.wait_text('manage-move', 'Move 2 people')
    assert b.js("return $('manage-all').indeterminate")

    # Moving: one department PUT each, and nothing until confirmed.
    b.js('window.asked = []; window.confirm = question => (asked.push(question), false)')
    b.js("$('manage-move-to').value = '10'")
    b.click('manage-move')
    assert b.js('return asked') == ['Move 2 people to Year 8?']
    b.js('window.confirm = () => true; net2.calls = []')
    b.click('manage-move')
    b.wait_starts('manage-notice', 'Done. 2 of 2 done.')
    writes = b.js("return net2.calls.map(c => [c.method, c.path, c.body])")
    assert writes == [
        ['PUT', '/users/8/departments', {'id': 10, 'name': 'Year 8'}],
        ['PUT', '/users/12/departments', {'id': 10, 'name': 'Year 8'}],
    ], writes
    assert [row[3] for row in b.js(f'return {MANAGED}')] == ['Moved to Year 8.', '', 'Moved to Year 8.']

    # Leavers: their department, then the whole record back with only the
    # expiry changed. Access that already ended is never given back.
    b.click('manage-all')
    b.js("$('manage-leavers-to').value = '11'; $('manage-leave-date').value = '2027-07-23'; net2.calls = []")
    b.click('manage-leave')
    b.wait_starts('manage-notice', 'Done. 3 of 3 done.')
    writes = b.js("return net2.calls.map(c => [c.method, c.path, c.body])")
    assert [w[:2] for w in writes] == [
        ['PUT', '/users/8/departments'], ['GET', '/users/8'], ['PUT', '/users/8'],
        ['PUT', '/users/9/departments'], ['GET', '/users/9'],
        ['PUT', '/users/12/departments'], ['GET', '/users/12'], ['PUT', '/users/12'],
    ], writes
    assert writes[2][2] == {
        'id': 8, 'firstName': 'Jane', 'lastName': 'Doe', 'hasImage': True,
        'expiryDate': '2027-07-23T23:59:00',
    }, writes[2]
    assert b.js(f'return {MANAGED}') == [
        ['Jane Doe', '8', '2027-07-23', 'Moved to Leavers. Access ends after 2027-07-23.'],
        ['Old Leaver', '9', '2020-07-31 (ended)',
         'Moved to Leavers. Access already ends sooner, so it was left as it is.'],
        ['John Roe', '12', '2027-07-23', 'Moved to Leavers. Access ends after 2027-07-23.'],
    ]

    # A refusal is shown on the person and does not stop the others.
    b.js("net2.failPath = '/users/8'; net2.calls = []")
    b.click('manage-leave')
    b.wait_starts('manage-notice', 'Done. 2 of 3 done.')
    assert b.js(f'return {MANAGED}')[0][3].startswith('Moved to Leavers. Access not ended: Net2 refused')
    assert b.js("return document.querySelector('#manage-rows tr td:last-child').className") == 'problem'
    b.js('delete net2.failPath')
    b.fits_desktop_and_phone('manage', 1000)

    # The year change: renamed from the top down, one new lowest year.
    b.js("$('manage-from').value = '7'; $('manage-to').value = '8'; $('manage-leavers-name').value = 'Leavers 2027'")
    b.js('window.asked = []; window.confirm = question => (asked.push(question), true)')
    b.click('manage-promote')
    b.wait_starts('manage-notice', 'Year change done: 3 changes.')
    assert b.js('return asked') == [
        'Make these 3 changes in Net2?\n• Year 8 becomes Leavers 2027\n• Year 7 becomes Year 8\n'
        '• a new, empty Year 7\nEveryone in each department moves with it.'
    ]
    assert b.js('return net2.departments') == [
        {'id': 3, 'name': 'Year 8'}, {'id': 4, 'name': 'Tusk test'}, {'id': 10, 'name': 'Leavers 2027'},
        {'id': 11, 'name': 'Leavers'}, {'id': 12, 'name': 'Staff'}, {'id': 50, 'name': 'Year 7'},
    ]
    assert 'Leavers 2027' in b.js("return [...$('manage-department').options].map(o => o.textContent)")
    # Running it again changes nothing.
    b.js('net2.calls = []')
    b.click('manage-promote')
    b.wait_starts('manage-notice', 'Nothing to change: Net2 already has Leavers 2027')
    assert b.js("return net2.calls.filter(c => c.method !== 'GET')") == []
    b.js('window.confirm = () => true')


def unplugging_and_browsers_without_webhid(b: Browser):
    b.js('reader.unplugged = true; reader.opened = false')
    b.wait_starts('message', 'Reader unplugged')
    assert b.js('return errors') == []
    # A browser with no WebHID still loads the page and explains the limitation.
    b.cdp('Page.addScriptToEvaluateOnNewDocument', {'source': 'delete navigator.hid; delete Navigator.prototype.hid;'})
    b.command('refresh', {})
    b.wait_starts('message', 'WebHID is not available')
    assert b.js("return $('connect').disabled")


def manage_release_workflows(b: Browser):
    b.js("""net2.calls = []; net2.persistManaged = true;
        net2.edited[8] = {id: 8, firstName: 'Jane', lastName: 'Doe', hasImage: true,
            pin: '1234', expiryDate: '2024-07-23T23:59:00'};
        net2.permissions[8] = {accessLevels: [1], individualPermissions: [{id: 42, timezoneId: 3}]};
        $('manage-department').value = '';""")
    b.click('manage-load')
    b.wait_contains('manage-notice', 'people in All users')
    b.js("[...$('manage-rows').rows].find(row => row.children[2].textContent === '8').querySelector('input').click()")
    assert b.js("return document.querySelector('#manage-rows tr:has(input:checked)').children[2].textContent") == '8'
    b.js("$('manage-undo-date').value = '2027-11-23'")
    b.click('manage-undo')
    b.wait_contains('manage-rows', 'Access ends after 2027-11-23.')
    b.js("$('manage-undo-date').value = ''")
    b.click('manage-undo')
    b.wait_contains('manage-rows', 'Expiry cleared.')
    assert b.js('return [net2.edited[8].pin, net2.edited[8].expiryDate]') == ['1234', None]

    b.click('manage-load-levels')
    b.wait_text('manage-notice', '2 access levels available.')
    b.js("$('manage-level').value = '2'")
    b.click('manage-set-level')
    b.wait_contains('manage-rows', 'Access level is now Car park.')
    assert b.js('return net2.permissions[8]') == {
        'accessLevels': [2], 'individualPermissions': [{'id': 42, 'timezoneId': 3}],
    }
    b.click('manage-remove-level')
    b.wait_contains('manage-rows', 'Removed Car park.')
    assert b.js('return net2.permissions[8]') == {
        'accessLevels': [], 'individualPermissions': [{'id': 42, 'timezoneId': 3}],
    }
    b.js('net2.ignoreLevels = true')
    b.click('manage-set-level')
    b.wait_contains('manage-rows', 'Net2 did not save the change to Car park.')
    assert b.js('return net2.permissions[8].accessLevels') == []
    b.js('net2.ignoreLevels = false')

    b.lift()
    signals = b.js('return reader.assignedSignals')
    b.tap('5B7D4062')
    b.wait_text('manage-card', 'Card 34935138 is on the reader.')
    b.js('window.confirm = () => false; net2.calls = []')
    b.click('manage-replace')
    assert b.js('return net2.calls') == []
    b.js('window.confirm = () => true')
    b.click('manage-replace')
    b.wait_contains('manage-rows', 'Saved card 34935138.')
    b.wait(f'reader.assignedSignals === {signals + 1}')
    b.lift()
    assert b.js('return net2.cards[8].at(-1)') == '34935138'
    assert b.js('return net2.cards[8].slice(0,-1).every(card => net2.lost.includes(card))')

    b.click('manage-activity')
    b.wait_contains('manage-site-status', 'Library. Showing 1 recent Net2 events.')
    b.wait_contains('manage-site-table', 'Access permitted')
    assert 'Wrong person' not in b.text('manage-site-table')
    b.click('manage-load-rollcalls')
    b.wait_starts('manage-site-status', '1 available.')
    b.js("$('manage-rollcall').value = '1'")
    b.click('manage-show-rollcall')
    b.wait_contains('manage-site-table', 'Main gate')
    b.wait_contains('manage-site-table', 'Steward')

    b.click('manage-load-unknown')
    b.wait_starts('manage-site-status', '1 available.')
    b.js("$('manage-unknown').value = '78901234'; net2.calls = []")
    b.click('manage-assign-unknown')
    b.wait_text('manage-site-status', 'Saved card 78901234 for Jane Doe.')
    assert b.js("return net2.calls.map(c => [c.method, c.path])") == [['POST', '/users/8/tokens']]
    assert b.js("return !net2.lost.includes('34935138')")

    b.click('manage-load-doors')
    b.wait_starts('manage-site-status', '1 available.')
    b.js("$('manage-door').value = '42'; net2.calls = []")
    b.click('manage-open-door')
    b.wait_text('manage-site-status', 'Net2 reported that the command failed.')
    b.click('manage-hold-door')
    b.wait_text('manage-site-status', 'Net2 completed the command.')
    b.click('manage-normal-door')
    b.wait_text('manage-site-status', 'Net2 completed the command.')
    assert b.js("return net2.calls.map(c => [c.method, c.path, c.body])") == [
        ['POST', '/commands/door/open', {'doorId': 42}],
        ['POST', '/commands/door/holdopen', {'doorId': 42}],
        ['POST', '/commands/door/reinstate', {'doorId': 42}],
    ]

    b.js("""net2.calls = [];
        document.querySelector('#manage-rows tr:has(input:checked) input').click();
        [...$('manage-rows').rows].find(row => row.children[2].textContent === '9').querySelector('input').click();
        $('manage-retention').value = '1'""")
    b.click('manage-preview-purge')
    b.wait_contains('manage-notice', '1 person eligible')
    assert b.js("$('manage-purge').click(); return $('manage-purge').disabled")
    assert b.js('return net2.calls') == []
    b.click('manage-download-purge')
    review = b.downloaded('net2-purge-review.csv')
    assert review[1][:2] == ['9', '2020-07-31'], review
    b.js("""net2.edited[9] = {id: 9, firstName: 'Old', lastName: 'Leaver',
        hasImage: false, expiryDate: null};""")
    b.click('manage-purge')
    b.wait_contains('manage-rows', 'User changed since the review.')
    assert b.js('return net2.purged') == []
    b.js('delete net2.edited[9]')
    b.click('manage-preview-purge', 'manage-download-purge', 'manage-purge')
    b.wait_contains('manage-rows', 'Purged user and history.')
    assert b.js('return net2.purged') == [9]


SCENARIOS = [
    reading_and_copying_a_card,
    batch_assign_from_a_spreadsheet,
    signing_in_to_net2,
    uploading_portraits,
    adding_people,
    keeping_people_import_results,
    saving_cards_straight_to_net2,
    reissuing_marks_old_cards_lost,
    looking_up_who_holds_a_card,
    reader_feedback_waits_for_the_saved_card,
    managing_people,
    manage_release_workflows,
    unplugging_and_browsers_without_webhid,
]


def main():
    dist = ROOT / 'dist'
    for name in ['index.html', 'tusk.js', 'tusk_bg.wasm', '.nojekyll']:
        assert (dist / name).is_file(), f'Build dist first: missing {name}'
    with serve(dist) as (url, scratch), chrome(scratch / 'downloads') as browser:
        browser.command('url', {'url': url})
        for scenario in SCENARIOS:
            scenario(browser)
            assert browser.js('return errors') == [], (scenario.__name__, browser.js('return errors'))
            print('passed:', scenario.__name__.replace('_', ' '))
    print(f'Static site smoke test passed: {len(SCENARIOS)} scenarios.')


if __name__ == '__main__':
    main()
