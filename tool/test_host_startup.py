"""The CLI must surface a host's early startup failure, without a readiness timeout."""
import os
from pathlib import Path
import subprocess
import tempfile
import time


def main():
    root = Path(__file__).resolve().parent.parent
    executable = root / 'target/debug' / ('thmr.exe' if os.name == 'nt' else 'thmr')
    with tempfile.TemporaryDirectory(prefix='thmr-startup-test-') as directory:
        temporary = Path(directory)
        config = temporary / 'config'
        config.mkdir()
        (config / 'settings.toml').write_text('volume = "not a number"\n', encoding='utf-8')
        env = dict(os.environ, THMR_RUNTIME_DIR=str(temporary / 'runtime'), THMR_CONFIG_DIR=str(config))
        started = time.monotonic()
        result = subprocess.run([str(executable), 'daemon', 'start'], env=env,
                                capture_output=True, timeout=20)
        elapsed = time.monotonic() - started
        stderr = result.stderr.decode('utf-8', errors='replace')
        assert result.returncode != 0
        assert 'invalid THMR settings' in stderr, f'Host failure was hidden: {stderr}'
        assert elapsed < 9, f'Early failure waited for readiness timeout: {elapsed:.1f}s'
        assert 'THMR host did not become ready' not in stderr
        print(f'PASS: startup error surfaced in {elapsed:.2f}s')


if __name__ == '__main__':
    main()
