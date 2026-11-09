"""Windows GUI first-frame receipt and shared host smoke in private runtime directories."""
import argparse
import http.server
import json
import os
from pathlib import Path
import secrets
import subprocess
import tempfile
import threading
import time


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('bundle', type=Path)
    args = parser.parse_args()
    if os.name != 'nt':
        parser.error('Use the existing native GTK/macOS GUI smoke tests on those platforms')
    bundle = args.bundle.resolve()
    class Metadata(http.server.BaseHTTPRequestHandler):
        def log_message(self, *_): pass
        def do_GET(self):
            body = json.dumps({'station': {'id': 'touhou', 'bitrates': [{'id': '128k', 'enabled': True}]},
                'current': {'trackId': 'test', 'title': '本地启动验证', 'artist': 'THMR', 'startedAtMs': 1}, 'queue': []}).encode()
            self.send_response(200)
            self.send_header('Content-Type', 'application/json')
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            self.wfile.write(body)
    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Metadata)
    server.daemon_threads = True
    threading.Thread(target=server.serve_forever, daemon=True).start()
    with tempfile.TemporaryDirectory(prefix='thmr-gui-test-') as directory:
        root = Path(directory)
        config = root / 'config'
        config.mkdir()
        origin = f'http://127.0.0.1:{server.server_port}'
        (config / 'settings.toml').write_text(f'source="source1"\nsource1="{origin}"\nsource2="{origin}"\n', encoding='utf-8')
        temporary = root / 'tmp'
        temporary.mkdir()
        token = secrets.token_hex(32)
        transaction = temporary / 'thmr-updates' / f'tx-{token}'
        transaction.mkdir(parents=True)
        ack = transaction / 'ack'
        (transaction / 'plan.json').write_text(json.dumps({'token': token, 'target': str(bundle)}))
        env = dict(os.environ, THMR_RUNTIME_DIR=str(root / 'runtime'), THMR_CONFIG_DIR=str(config),
                   THMR_AUDIO_OUTPUT='null', TEMP=str(temporary), TMP=str(temporary))
        cli = bundle / 'thmr.exe'
        with (root / 'gui.log').open('w+', encoding='utf-8') as log:
            process = subprocess.Popen([str(bundle / 'thmr_app.exe'), '--thmr-update-ack', str(ack),
                '--thmr-update-token', token], env=env, stdout=log, stderr=log,
                creationflags=subprocess.CREATE_NO_WINDOW)
            try:
                deadline = time.monotonic() + 45
                while not ack.exists() and time.monotonic() < deadline:
                    if process.poll() is not None:
                        raise AssertionError('GUI exited before its healthy first-frame receipt; check for an existing GUI instance')
                    time.sleep(0.1)
                assert ack.exists() and ack.read_text() == token, 'missing healthy GUI startup receipt'
                result = subprocess.run([str(cli), 'status', '--json'], env=env, capture_output=True, encoding='utf-8', timeout=10)
                assert result.returncode == 0, result.stderr
                before = json.loads(result.stdout)['instanceId']
                registered = json.loads((config / 'gui.json').read_text())
                assert registered['protocolVersion'] == 1
                subprocess.run([str(cli), 'gui'], env=env, check=True, capture_output=True, timeout=10)
                assert process.poll() is None
                subprocess.run([str(cli), 'volume', '37'], env=env, check=True, capture_output=True, timeout=10)
                process.terminate()
                process.wait(timeout=10)
                result = subprocess.run([str(cli), 'status', '--json'], env=env, capture_output=True, encoding='utf-8', timeout=10)
                state = json.loads(result.stdout)
                assert state['instanceId'] == before and state['volume'] == 37
                print('PASS: Flutter healthy first frame, shared host/CLI settings, abrupt GUI termination preserves host')
            except Exception:
                log.seek(0)
                print(log.read())
                raise
            finally:
                if process.poll() is None:
                    process.terminate()
                    process.wait(timeout=10)
                subprocess.run([str(cli), 'daemon', 'stop'], env=env, capture_output=True, timeout=10)
                for _ in range(80):
                    if not (root / 'runtime/instance.json').exists(): break
                    time.sleep(0.1)
        server.shutdown()


if __name__ == '__main__':
    main()
