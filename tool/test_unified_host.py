"""Exercise the real host, libmpv, CLI and loopback contract against local radio audio.

Uses the null audio output and isolated runtime/settings; never changes the user's host.
Run after cargo build --workspace. THMR_MPV_LIBRARY can select a development library.
"""
import argparse
import http.server
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import threading
import time
import urllib.request
import urllib.error


def wait_for(check, timeout=15):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        result = check()
        if result:
            return result
        time.sleep(0.1)
    raise AssertionError('timed out waiting for host state')


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--bin-dir', default='target/debug')
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    bin_dir = (root / args.bin_dir).resolve()
    suffix = '.exe' if os.name == 'nt' else ''
    host, cli = bin_dir / ('thmrd' + suffix), bin_dir / ('thmr' + suffix)
    audio = (root / 'test/fixtures/sine_440hz_48000_mono.mp3').read_bytes()
    active = 0
    maximum = 0
    mutex = threading.Lock()

    class Radio(http.server.BaseHTTPRequestHandler):
        protocol_version = 'HTTP/1.1'
        def log_message(self, *_):
            pass
        def do_GET(self):
            nonlocal active, maximum
            if self.path.endswith('/status'):
                status = {'station': {'id': 'touhou', 'name': 'Fixture', 'enabled': True,
                          'bitrates': [{'id': '128k', 'enabled': True}, {'id': '96k', 'enabled': True}]},
                          'current': {'trackId': 'fixture', 'title': '本地测试', 'artist': 'THMR', 'coverUrl': '',
                                      'startedAtMs': 1, 'expectedEndMs': 0, 'positionSec': 0, 'durationSec': 0},
                          'queue': [], 'totals': {'listeners': 1}}
                body = json.dumps(status).encode()
                self.send_response(200)
                self.send_header('Content-Type', 'application/json')
                self.send_header('Content-Length', str(len(body)))
                self.end_headers()
                self.wfile.write(body)
                return
            if not self.path.startswith('/radio/'):
                self.send_error(404)
                return
            self.send_response(200)
            self.send_header('Content-Type', 'audio/mpeg')
            self.send_header('Connection', 'close')
            self.end_headers()
            with mutex:
                active += 1
                maximum = max(maximum, active)
            try:
                for _ in range(600):
                    self.wfile.write(audio)
                    self.wfile.flush()
                    time.sleep(0.08)
            except (OSError, ConnectionError):
                pass
            finally:
                with mutex:
                    active -= 1

    radio = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Radio)
    radio.daemon_threads = True
    threading.Thread(target=radio.serve_forever, daemon=True).start()
    with tempfile.TemporaryDirectory(prefix='thmr-host-test-') as directory:
        temp = Path(directory)
        config = temp / 'config'
        config.mkdir()
        url = f'http://127.0.0.1:{radio.server_port}'
        (config / 'settings.toml').write_text(f'source="source1"\nsource1="{url}"\nsource2="{url}"\n', encoding='utf-8')
        env = dict(os.environ, THMR_RUNTIME_DIR=str(temp / 'runtime'), THMR_CONFIG_DIR=str(config), THMR_AUDIO_OUTPUT='null')
        if os.name == 'nt' and not env.get('THMR_MPV_LIBRARY'):
            library = root / 'build/windows/x64/runner/Release/libmpv-2.dll'
            if library.exists():
                env['THMR_MPV_LIBRARY'] = str(library)
        log = (temp / 'host.log').open('w', encoding='utf-8')
        process = subprocess.Popen([str(host)], env=env, stdout=log, stderr=log)
        endpoint = None
        def state():
            nonlocal endpoint
            try:
                endpoint = json.loads((temp / 'runtime/instance.json').read_text())
                request = urllib.request.Request(f"http://127.0.0.1:{endpoint['port']}/v1/state",
                    headers={'Authorization': 'Bearer ' + endpoint['token'], 'X-THMR-Protocol': '1'})
                with urllib.request.build_opener(urllib.request.ProxyHandler({})).open(request, timeout=2) as response:
                    return json.load(response)
            except (OSError, ValueError):
                return None
        def command(*args, success=True):
            result = subprocess.run([str(cli), *args], env=env, capture_output=True, text=True, encoding='utf-8', timeout=15)
            if success:
                assert result.returncode == 0, result.stderr
            else:
                assert result.returncode != 0
            return result
        try:
            wait_for(state)
            wait_for(lambda: state().get('stationStatus'))
            command('play')
            wait_for(lambda: state()['phase'] == 'online')
            duplicate = subprocess.run([str(host)], env=env, capture_output=True, timeout=10)
            assert duplicate.returncode != 0
            assert state()['phase'] == 'online'
            command('volume', '35')
            assert state()['volume'] == 35
            command('volume', '101', success=False)
            assert state()['volume'] == 35
            command('pause')
            wait_for(lambda: active == 0)
            command('quality', '96k')
            command('source', 'source2')
            time.sleep(0.5)
            paused = state()
            assert paused['phase'] == 'idle' and paused['userPaused'] and paused['profileId'] == '96k'
            assert active == 0
            output = command('status', '--json').stdout
            assert endpoint['token'] not in output
            command('play')
            wait_for(lambda: state()['phase'] == 'online')
            command('stop')
            wait_for(lambda: active == 0)
            command('daemon', 'prepare-update')
            process.wait(timeout=10)
            assert process.returncode == 0
            assert not (temp / 'runtime/instance.json').exists()
            command('status', '--json', success=False)
            command('play')  # CLI autostart uses the same isolated configuration.
            wait_for(lambda: state() and state()['phase'] == 'online')
            assert state()['volume'] == 35 and state()['profileId'] == '96k'
            command('daemon', 'stop')
            wait_for(lambda: not (temp / 'runtime/instance.json').exists())
            assert maximum == 1, f'duplicate remote audio connections: {maximum}'
            print('PASS: real libmpv, live-edge playback, pause/source/profile, persistence, single owner, CLI autostart/shutdown')
        finally:
            if process.poll() is None:
                process.terminate()
                process.wait(timeout=10)
            try:
                if state():
                    command('daemon', 'stop')
                    wait_for(lambda: not (temp / 'runtime/instance.json').exists())
            except Exception:
                pass
            log.close()
            if process.returncode:
                print((temp / 'host.log').read_text(encoding='utf-8'))
            radio.shutdown()


if __name__ == '__main__':
    main()
