import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import zipfile

spec = importlib.util.spec_from_file_location('package_desktop', Path(__file__).with_name('package_desktop.py'))
package = importlib.util.module_from_spec(spec)
spec.loader.exec_module(package)


class TerminalPackaging(unittest.TestCase):
    @staticmethod
    def version_output(args, **_):
        return 'THMR 0.2.0-dev.1+1 · protocol 1\n' if 'thmrd' in args[0] else 'thmr 0.2.0-dev.1\n'
    def test_windows_terminal_contains_real_audio_dependency_and_no_flutter(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            native = root / 'native'
            bundle = root / 'gui'
            (root / 'docs').mkdir()
            (root / 'docs/TERMINAL.md').write_text('terminal instructions')
            native.mkdir()
            (root / 'native/vendor/minimp3').mkdir(parents=True)
            (root / 'native/vendor/minimp3/LICENSE').write_text('minimp3 CC0')
            bundle.mkdir()
            for name in ['thmr.exe', 'thmrd.exe']:
                (native / name).write_bytes(name.encode())
            (bundle / 'libmpv-2.dll').write_bytes(b'pinned mpv')
            (bundle / 'flutter_windows.dll').write_bytes(b'not part of terminal package')
            with patch.object(package, 'ROOT', root), patch.object(package.subprocess, 'check_output', side_effect=self.version_output):
                output = package.stage('windows', bundle, native)
                self.assertEqual((output / 'bin/libmpv-2.dll').read_bytes(), b'pinned mpv')
                self.assertFalse((output / 'flutter_windows.dll').exists())
                self.assertEqual((output / 'licenses/minimp3/LICENSE').read_text(), 'minimp3 CC0')
                self.assertEqual((bundle / 'licenses/minimp3/LICENSE').read_text(), 'minimp3 CC0')
                self.assertEqual(json.loads((output / 'native-version.json').read_text())['protocolVersion'], 1)
                archive = package.archive('windows')
                with zipfile.ZipFile(archive) as files:
                    self.assertIn('bin/thmr.exe', files.namelist())
                    self.assertIn('bin/libmpv-2.dll', files.namelist())
                    self.assertIn('licenses/minimp3/LICENSE', files.namelist())

    def test_linux_root_command_is_cli_and_gui_launcher_is_preserved(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            native = root / 'native'
            bundle = root / 'gui'
            (root / 'docs').mkdir()
            (root / 'docs/TERMINAL.md').write_text('terminal instructions')
            native.mkdir()
            (root / 'native/vendor/minimp3').mkdir(parents=True)
            (root / 'native/vendor/minimp3/LICENSE').write_text('minimp3 CC0')
            bundle.mkdir()
            for name in ['thmr', 'thmrd']:
                (native / name).write_bytes(name.encode())
            (bundle / 'thmr-gui').write_text('# GUI launcher\n')
            (bundle / 'thmr_app').write_bytes(b'Flutter GUI executable')
            with patch.object(package, 'ROOT', root), patch.object(package.subprocess, 'check_output', side_effect=self.version_output):
                output = package.stage('linux', bundle, native)
                self.assertEqual((bundle / 'thmr').read_bytes(), b'thmr')
                self.assertEqual((bundle / 'thmrd').read_bytes(), b'thmrd')
                self.assertEqual((bundle / 'thmr-gui').read_text(), '# GUI launcher\n')
                self.assertEqual((bundle / 'thmr_app').read_bytes(), b'Flutter GUI executable')
                self.assertFalse((bundle / 'bin/thmr').exists())
                self.assertEqual((output / 'bin/thmr').read_bytes(), b'thmr')
                self.assertTrue((output / 'bin/thmrd').exists())
                # Terminal-only restaging must use the same root native programs.
                output = package.stage('linux', bundle, native, terminal_only=True)
                self.assertEqual((output / 'bin/thmr').read_bytes(), b'thmr')

    def test_macos_helpers_preserve_case_insensitive_gui_executable(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            native = root / 'native'
            bundle = root / 'THMR.app'
            (root / 'docs').mkdir()
            (root / 'docs/TERMINAL.md').write_text('terminal instructions')
            native.mkdir()
            (root / 'native/vendor/minimp3').mkdir(parents=True)
            (root / 'native/vendor/minimp3/LICENSE').write_text('minimp3 CC0')
            gui = bundle / 'Contents/MacOS/THMR'
            gui.parent.mkdir(parents=True)
            gui.write_bytes(b'Flutter GUI executable')
            audio = bundle / 'Contents/Frameworks/Mpv.framework/Mpv'
            audio.parent.mkdir(parents=True)
            audio.write_bytes(b'pinned audio')
            for name in ['thmr', 'thmrd']:
                (native / name).write_bytes(name.encode())
            def output(args, **kwargs):
                if args[0] == 'otool':
                    return 'Mpv:\n\t/usr/lib/libSystem.B.dylib (compatibility version 1.0)\n'
                return self.version_output(args, **kwargs)
            with patch.object(package, 'ROOT', root), patch.object(package.subprocess, 'check_output', side_effect=output):
                terminal = package.stage('macos', bundle, native)
                self.assertEqual(gui.read_bytes(), b'Flutter GUI executable')
                self.assertEqual((bundle / 'Contents/Helpers/thmr').read_bytes(), b'thmr')
                self.assertTrue((bundle / 'Contents/Helpers/thmrd').exists())
                self.assertTrue((terminal / 'THMRHost.app/Contents/Frameworks/Mpv.framework/Mpv').exists())


if __name__ == '__main__':
    unittest.main()
