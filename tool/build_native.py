"""Build the version-matched native programs for the current desktop platform."""
import argparse
import os
from pathlib import Path
import platform
import subprocess

ROOT = Path(__file__).resolve().parent.parent


def run(*args, env=None):
    subprocess.run(args, cwd=ROOT, env=env, check=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--version')
    parser.add_argument('--build-number')
    parser.add_argument('--debug', action='store_true')
    args = parser.parse_args()
    env = dict(os.environ)
    if args.version:
        env['THMR_BUILD_VERSION'] = args.version
    if args.build_number:
        env['THMR_BUILD_NUMBER'] = args.build_number
    profile = 'debug' if args.debug else 'release'
    cargo = ['cargo', 'build', '--workspace', '--locked']
    if not args.debug:
        cargo.append('--release')
    if platform.system() == 'Darwin':
        env['RUSTFLAGS'] = (env.get('RUSTFLAGS', '') +
            ' -C link-arg=-Wl,-rpath,@executable_path/../Frameworks').strip()
        for target, minimum in [('x86_64-apple-darwin', '10.15'), ('aarch64-apple-darwin', '11.0')]:
            run('rustup', 'target', 'add', target)
            target_env = dict(env, MACOSX_DEPLOYMENT_TARGET=minimum)
            run(*cargo, '--target', target, env=target_env)
        output = ROOT / 'target/universal' / profile
        output.mkdir(parents=True, exist_ok=True)
        for name in ['thmr', 'thmrd']:
            run('lipo', '-create', str(ROOT / 'target/x86_64-apple-darwin' / profile / name),
                str(ROOT / 'target/aarch64-apple-darwin' / profile / name), '-output', str(output / name))
    elif platform.system() == 'Windows':
        target = 'x86_64-pc-windows-msvc'
        # Terminal users should not need the Visual C++ redistributable for Rust.
        env['RUSTFLAGS'] = (env.get('RUSTFLAGS', '') + ' -C target-feature=+crt-static').strip()
        run('rustup', 'target', 'add', target)
        run(*cargo, '--target', target, env=env)
    else:
        run(*cargo, env=env)


if __name__ == '__main__':
    main()
