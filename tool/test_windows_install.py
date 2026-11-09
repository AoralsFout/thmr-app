"""Exercise Windows installation rules with fresh CMake caches."""

from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


CMAKE = shutil.which('cmake')
ROOT = Path(__file__).resolve().parent.parent


@unittest.skipUnless(CMAKE, 'CMake is required for Windows install tests')
class WindowsInstallTests(unittest.TestCase):
    def test_first_configuration_sets_prefix_before_runner_install(self):
        project = (ROOT / 'windows/CMakeLists.txt').read_text(encoding='utf-8')
        # Keep the real ordering of the prefix setup and the runner call.
        commands = [line for line in project.splitlines() if
                    line.startswith(('set(BUILD_BUNDLE_DIR ', 'set(CMAKE_INSTALL_PREFIX '))
                    or line == 'add_subdirectory("runner")']
        self.assertEqual(len(commands), 3)
        with tempfile.TemporaryDirectory(prefix='thmr first install ') as temporary:
            root = Path(temporary)
            source = root / 'source'
            runner = source / 'runner'
            runner.mkdir(parents=True)
            build = root / 'cmake build'
            bundle = build / 'runner/Debug'
            (source / 'CMakeLists.txt').write_text(f'''
cmake_minimum_required(VERSION 3.14)
project(FirstInstallTest NONE)
set(BINARY_NAME thmr_app)
add_executable(thmr_app IMPORTED GLOBAL)
set_target_properties(thmr_app PROPERTIES
    IMPORTED_LOCATION "{bundle.as_posix()}/thmr_app.exe")
''' + '\n'.join(commands), encoding='utf-8')
            (runner / 'CMakeLists.txt').write_text('''
file(WRITE "${CMAKE_CURRENT_BINARY_DIR}/thmr_capture.dll" "capture fixture")
install(FILES "${CMAKE_CURRENT_BINARY_DIR}/thmr_capture.dll"
    DESTINATION "${CMAKE_INSTALL_PREFIX}" COMPONENT Runtime)
''', encoding='utf-8')
            configured = subprocess.run([CMAKE, '-S', str(source), '-B', str(build)],
                                        capture_output=True, text=True, timeout=30)
            self.assertEqual(configured.returncode, 0, configured.stdout + configured.stderr)
            install_script = (build / 'runner/cmake_install.cmake').read_text(encoding='utf-8')
            # Check the destination before allowing the install to write anything.
            self.assertIn(f'file(INSTALL DESTINATION "{bundle.as_posix()}"', install_script)
            installed = subprocess.run([CMAKE, '--install', str(build), '--config', 'Debug'],
                                       capture_output=True, text=True, timeout=30)
            self.assertEqual(installed.returncode, 0, installed.stdout + installed.stderr)
            self.assertEqual((bundle / 'thmr_capture.dll').read_bytes(), b'capture fixture')

    def native_assets_scenario(self, asset=None):
        with tempfile.TemporaryDirectory(prefix='thmr native assets ') as temporary:
            root = Path(temporary)
            source = root / 'source'
            source.mkdir()
            assets = root / 'flutter build'
            bundle = root / 'bundle'
            project = (ROOT / 'windows/CMakeLists.txt').read_text(encoding='utf-8')
            start = project.index('set(NATIVE_ASSETS_DIR ')
            end = project.index('\n\n', start)
            install_rule = project[start:end]
            (source / 'CMakeLists.txt').write_text(f'''
cmake_minimum_required(VERSION 3.14)
project(NativeAssetsInstallTest NONE)
set(PROJECT_BUILD_DIR "{assets.as_posix()}/")
set(INSTALL_BUNDLE_LIB_DIR "${{CMAKE_INSTALL_PREFIX}}")
{install_rule}
''', encoding='utf-8')
            configured = subprocess.run(
                [CMAKE, '-S', str(source), '-B', str(root / 'cmake build'),
                 f'-DCMAKE_INSTALL_PREFIX={bundle.as_posix()}'],
                capture_output=True, text=True, timeout=30)
            self.assertEqual(configured.returncode, 0, configured.stdout + configured.stderr)
            # Assets can appear during the build, after CMake configuration.
            if asset is not None:
                native_assets = assets / 'native_assets/windows'
                native_assets.mkdir(parents=True)
                (native_assets / 'fixture.dll').write_bytes(asset)
            installed = subprocess.run(
                [CMAKE, '--install', str(root / 'cmake build'), '--config', 'Debug'],
                capture_output=True, text=True, timeout=30)
            copied = bundle / 'fixture.dll'
            return installed, copied.read_bytes() if copied.exists() else None

    def test_missing_native_assets_directory_does_not_fail_install(self):
        result, copied = self.native_assets_scenario()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIsNone(copied)

    def test_native_assets_created_after_configuration_are_copied(self):
        result, copied = self.native_assets_scenario(b'native asset fixture')
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(copied, b'native asset fixture')


if __name__ == '__main__':
    unittest.main()
