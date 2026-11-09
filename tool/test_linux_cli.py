#!/usr/bin/env python3
"""Exercise the packaged CLI's GUI routing without audio or a graphical session."""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]


def exercise(cli):
    with tempfile.TemporaryDirectory(prefix='thmr CLI 中文 ') as temporary:
        root = Path(temporary)
        bundle = root / 'bundle with spaces'
        bundle.mkdir()
        shutil.copy2(cli, bundle / 'thmr')
        shutil.copy2(ROOT / 'tool/linux/launch.sh', bundle / 'thmr-gui')
        (bundle / 'thmr-gui').chmod(0o755)
        probe = root / 'gui-arguments'
        checked = root / 'runtime-checked'
        for name, body in [
            ('thmr_app', 'printf "%s\\n" "$@" > "$THMR_LAUNCH_PROBE"\n'),
            ('thmr_runtime_check', 'printf checked > "$THMR_CHECK_PROBE"\n'),
            ('ldd', 'exit 0\n'),
        ]:
            path = bundle / name
            path.write_text('#!/bin/sh\n' + body)
            path.chmod(0o755)
        (bundle / 'native-version.json').write_text(json.dumps({'protocolVersion': 1}))
        config = root / 'config'
        config.mkdir()
        env = dict(os.environ, THMR_CONFIG_DIR=str(config),
                   THMR_RUNTIME_DIR=str(root / 'runtime'),
                   THMR_LAUNCH_PROBE=str(probe), THMR_CHECK_PROBE=str(checked),
                   PATH=f'{bundle}:{os.environ["PATH"]}')
        env.pop('THMR_GUI_PATH', None)
        command = str(bundle / 'thmr')
        # Cover fresh discovery and registrations written before/after migration.
        for registered in [None, 'thmr', 'thmr_app', 'thmr-gui']:
            registration = config / 'gui.json'
            if registered:
                registration.write_text(json.dumps({
                    'protocolVersion': 1, 'executable': str(bundle / registered)}))
            result = subprocess.run([command, 'gui'], env=env, capture_output=True,
                                    text=True, timeout=10)
            assert result.returncode == 0, result.stderr
            deadline = time.monotonic() + 5
            while time.monotonic() < deadline:
                if probe.exists() and probe.read_text() == '--thmr-daemon\n':
                    break
                time.sleep(0.02)
            else:
                raise AssertionError(f'GUI was not launched for registration {registered!r}')
            assert checked.read_text() == 'checked', 'GUI bypassed the runtime check'
            probe.unlink()
            checked.unlink()
        result = subprocess.run([command], env=env, capture_output=True,
                                text=True, timeout=10)
        assert result.returncode != 0 and 'subcommand' in result.stderr, result.stderr
        assert not probe.exists(), 'Non-interactive CLI unexpectedly launched a GUI'
        print('CLI routing passed: fresh bundle, legacy/current GUI registrations, '
              'non-interactive command selection')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('cli', type=Path)
    args = parser.parse_args()
    exercise(args.cli.resolve())


if __name__ == '__main__':
    main()
