"""Add native programs to the GUI and build a separate Flutter-free terminal package."""
import argparse
import json
import os
from pathlib import Path
import plistlib
import re
import shutil
import subprocess
import tarfile
import zipfile

ROOT = Path(__file__).resolve().parent.parent


def native_directory(system):
    return ROOT / {'windows': 'target/x86_64-pc-windows-msvc/release',
                   'linux': 'target/release', 'macos': 'target/universal/release'}[system]


def stage(system, bundle, native, terminal_only=False):
    bundle = bundle.resolve()
    output = (ROOT / 'build/terminal' / system).resolve()
    if not output.is_relative_to((ROOT / 'build/terminal').resolve()) or output.is_symlink():
        raise ValueError('Unexpected terminal staging directory')
    if output.exists():
        shutil.rmtree(output)
    output.mkdir(parents=True)
    # macOS commonly uses a case-insensitive filesystem: "thmr" would replace
    # the Flutter executable named "THMR" if copied beside it.
    destination = bundle / 'Contents/Helpers' if system == 'macos' else bundle
    suffix = '.exe' if system == 'windows' else ''
    if not terminal_only:
        destination.mkdir(parents=True, exist_ok=True)
        for name in ['thmr', 'thmrd']:
            shutil.copy2(native / (name + suffix), destination / (name + suffix))
    version = subprocess.check_output([str(destination / ('thmr' + suffix)), '--version'], encoding='utf-8').strip().split()[-1]
    host_version = subprocess.check_output([str(destination / ('thmrd' + suffix)), '--version'], encoding='utf-8').split()[1]
    host_product, build = host_version.rsplit('+', 1)
    if host_product != version: raise ValueError('CLI and host versions differ')
    metadata = {'version': version, 'buildNumber': build, 'protocolVersion': 1}
    info_dir = bundle / 'Contents/Resources' if system == 'macos' else bundle
    info_dir.mkdir(parents=True, exist_ok=True)
    if not terminal_only:
        (info_dir / 'native-version.json').write_text(json.dumps(metadata), encoding='utf-8')
    binaries = output / 'bin'
    binaries.mkdir()
    shutil.copy2(destination / ('thmr' + suffix), binaries / ('thmr' + suffix))
    if system == 'macos':
        host = output / 'THMRHost.app/Contents'
        (host / 'MacOS').mkdir(parents=True)
        shutil.copy2(destination / 'thmrd', host / 'MacOS/thmrd')
        with (host / 'Info.plist').open('wb') as stream:
            plistlib.dump({'CFBundleIdentifier': 'cn.yunmoan.thmr.host', 'CFBundleName': 'THMR',
                'CFBundleExecutable': 'thmrd', 'CFBundlePackageType': 'APPL', 'LSUIElement': True,
                'CFBundleShortVersionString': version.split('-')[0], 'CFBundleVersion': os.environ.get('CLIENT_BUILD_NUMBER', '1'),
                'NSHighResolutionCapable': True, 'NSPrincipalClass': 'NSApplication'}, stream)
        frameworks = host / 'Frameworks'
        frameworks.mkdir()
        # Follow the actual audio dependency graph; never bundle Flutter to satisfy a guess.
        available = bundle / 'Contents/Frameworks'
        pending = ['Mpv.framework']
        copied = set()
        while pending:
            name = pending.pop()
            if name in copied: continue
            if name in ['App.framework', 'FlutterMacOS.framework']:
                raise ValueError('Audio dependency unexpectedly requires Flutter')
            source = available / name
            if not source.exists(): raise ValueError(f'Missing audio dependency: {name}')
            if source.is_dir():
                shutil.copytree(source, frameworks / name, symlinks=True)
                binary = source / source.stem
            else:
                shutil.copy2(source, frameworks / name, follow_symlinks=False)
                binary = source
            copied.add(name)
            dependencies = subprocess.check_output(['otool', '-L', str(binary)], text=True).splitlines()[1:]
            for line in dependencies:
                dependency = line.strip().split(' (', 1)[0]
                if dependency.startswith(('/System/', '/usr/lib/')): continue
                match = re.search(r'([^/]+\.framework)(?:/|$)', dependency)
                local = match.group(1) if match else Path(dependency).name
                if local != name: pending.append(local)
        if not (frameworks / 'Mpv.framework').exists():
            raise ValueError('Missing Mpv.framework in macOS GUI bundle')
    else:
        shutil.copy2(destination / ('thmrd' + suffix), binaries / ('thmrd' + suffix))
        if system == 'windows':
            shutil.copy2(bundle / 'libmpv-2.dll', binaries / 'libmpv-2.dll')
    shutil.copy2(ROOT / 'docs/TERMINAL.md', output / 'README.md')
    (output / 'native-version.json').write_text(json.dumps(metadata), encoding='utf-8')
    # Include the dependency notices shipped with the pinned GUI build where present.
    for source in [bundle / 'data/flutter_assets/NOTICES.Z', bundle / 'Contents/Frameworks/App.framework/Resources/flutter_assets/NOTICES.Z']:
        if source.exists(): shutil.copy2(source, output / 'NOTICES.Z')
    return output


def archive(system):
    output = ROOT / 'build/terminal' / system
    name = {'windows': 'thmr-terminal-windows-x64.zip', 'linux': 'thmr-terminal-linux-x64.tar.gz',
            'macos': 'thmr-terminal-macos-universal.tar.gz'}[system]
    target = ROOT / name
    if system == 'windows':
        with zipfile.ZipFile(target, 'w', zipfile.ZIP_DEFLATED) as package:
            for file in sorted(output.rglob('*')):
                if file.is_file(): package.write(file, file.relative_to(output))
    else:
        with tarfile.open(target, 'w:gz') as package:
            package.add(output, arcname='thmr-terminal')
    print(target)
    return target


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--platform', choices=['windows', 'linux', 'macos'], required=True)
    parser.add_argument('--bundle', type=Path)
    parser.add_argument('--rust-dir', type=Path)
    parser.add_argument('--terminal-only', action='store_true')
    parser.add_argument('--archive-only', action='store_true')
    parser.add_argument('--stage-only', action='store_true')
    args = parser.parse_args()
    if not args.archive_only:
        if not args.bundle: parser.error('--bundle is required for staging')
        stage(args.platform, args.bundle, args.rust_dir or native_directory(args.platform), args.terminal_only)
    if not args.stage_only:
        archive(args.platform)


if __name__ == '__main__':
    main()
