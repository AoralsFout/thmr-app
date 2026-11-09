"""Build the version-matched native programs for the current desktop platform."""
import argparse
import filecmp
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import tempfile
from urllib.parse import urljoin, urlparse
from urllib.request import Request, url2pathname, urlopen

ROOT = Path(__file__).resolve().parent.parent


def run(*args, env=None):
    subprocess.run(args, cwd=ROOT, env=env, check=True)


def windows_mpv_pin():
    """Use the resolved Flutter plugin's audio artifact and integrity pin."""
    config = ROOT / '.dart_tool/package_config.json'
    if not config.is_file():
        raise RuntimeError('Run flutter pub get before building the Windows audio runtime')
    packages = json.loads(config.read_text(encoding='utf-8'))['packages']
    package = next((p for p in packages if p['name'] == 'media_kit_libs_windows_audio'), None)
    if package is None:
        raise RuntimeError('media_kit_libs_windows_audio is missing; run flutter pub get')
    uri = urlparse(urljoin(config.as_uri(), package['rootUri']))
    if uri.scheme != 'file' or uri.netloc:
        raise ValueError('Expected a local media_kit_libs_windows_audio package')
    source = (Path(url2pathname(uri.path)) / 'windows/CMakeLists.txt').read_text(encoding='utf-8')
    pins = {}
    for key in ['LIBMPV', 'LIBMPV_URL', 'LIBMPV_MD5']:
        match = re.search(r'\bset\(\s*' + key + r'\s+"([^"]+)"\s*\)', source)
        if match is None:
            raise ValueError(f'Cannot read {key} from the resolved audio plugin')
        pins[key] = match[1]
    name = pins['LIBMPV']
    url = pins['LIBMPV_URL'].replace('${LIBMPV}', name)
    digest = pins['LIBMPV_MD5'].lower()
    if not re.fullmatch(r'[\w.-]+\.7z', name) or not re.fullmatch(r'[a-f0-9]{32}', digest):
        raise ValueError('Invalid audio artifact name or checksum')
    if not url.startswith('https://') or '${' in url:
        raise ValueError('Expected an HTTPS audio artifact URL')
    return name, url, digest


def github_asset_url(url, name):
    """Use GitHub's official asset endpoint when the release download gateway fails."""
    parsed = urlparse(url)
    match = re.fullmatch(r'/([^/]+)/([^/]+)/releases/download/([^/]+)/[^/]+', parsed.path)
    if parsed.hostname != 'github.com' or match is None:
        return url
    owner, repository, tag = match.groups()
    api = f'https://api.github.com/repos/{owner}/{repository}/releases/tags/{tag}'
    with urlopen(Request(api, headers={'User-Agent': 'THMR-native-build'}), timeout=60) as response:
        release = json.load(response)
    for asset in release.get('assets', []):
        asset_url = asset.get('url', '')
        if asset.get('name') == name and asset_url.startswith('https://api.github.com/'):
            return asset_url
    raise ValueError('The pinned audio asset was not found in its GitHub release')


def stage_windows_mpv(output):
    name, url, digest = windows_mpv_pin()
    cmake = shutil.which('cmake')
    system_tar = Path(os.environ.get('SystemRoot', 'C:/Windows')) / 'System32/tar.exe'
    if cmake:
        extractor = [cmake, '-E', 'tar', 'xf']
    elif system_tar.is_file():
        extractor = [str(system_tar), '-xf']
    else:
        raise RuntimeError('CMake or Windows tar.exe is required to extract the audio runtime')
    cache = ROOT / 'build/native/windows'
    cache.mkdir(parents=True, exist_ok=True)
    archive = cache / name
    # A Flutter build may already have downloaded exactly the same artifact.
    for candidate in [archive, ROOT / 'build/windows/x64' / name]:
        if candidate.is_file() and hashlib.md5(candidate.read_bytes()).hexdigest() == digest:
            archive = candidate
            break
    else:
        archive.unlink(missing_ok=True)
        download_url = url
        for attempt in range(1, 4):
            print(f'Downloading Windows audio runtime (attempt {attempt}/3): {url}', flush=True)
            with tempfile.TemporaryDirectory(prefix='mpv-download-', dir=cache) as temporary:
                download = Path(temporary) / name
                try:
                    if attempt > 1 and download_url == url:
                        download_url = github_asset_url(url, name)
                    request = Request(download_url, headers={
                        'User-Agent': 'THMR-native-build', 'Accept': 'application/octet-stream'})
                    with urlopen(request, timeout=60) as response, download.open('wb') as stream:
                        shutil.copyfileobj(response, stream)
                    if hashlib.md5(download.read_bytes()).hexdigest() != digest:
                        raise ValueError('Windows audio runtime checksum mismatch')
                    download.replace(archive)
                    break
                except (OSError, ValueError) as error:
                    if attempt == 3:
                        raise RuntimeError(f'Could not download the pinned Windows audio runtime: {error}') from error
    with tempfile.TemporaryDirectory(prefix='mpv-extract-', dir=cache) as temporary:
        subprocess.run([*extractor, str(archive), 'libmpv-2.dll'],
                       cwd=temporary, check=True)
        library = Path(temporary) / 'libmpv-2.dll'
        if not library.is_file():
            raise RuntimeError('The pinned audio archive does not contain libmpv-2.dll')
        output.mkdir(parents=True, exist_ok=True)
        destination = output / library.name
        # Do not rewrite an identical DLL that a running development host may hold open.
        if not destination.is_file() or not filecmp.cmp(library, destination, shallow=False):
            shutil.copy2(library, destination)
    print(f'Windows audio runtime ready: {destination}', flush=True)


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
        output = ROOT / 'target' / target / profile
        stage_windows_mpv(output)
        check_env = dict(env)
        check_env.pop('THMR_MPV_LIBRARY', None)
        run(str(output / 'thmrd.exe'), '--check', env=check_env)
    else:
        run(*cargo, env=env)


if __name__ == '__main__':
    main()
