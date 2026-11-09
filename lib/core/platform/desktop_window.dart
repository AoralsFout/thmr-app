import 'dart:async';
import 'dart:io' show Platform, exit;

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart' show Colors, Size;
import 'package:window_manager/window_manager.dart';

import 'windows_process_exit.dart';
import '../player/desktop_session.dart';

bool get hasDesktopWindowControls =>
    !kIsWeb && (Platform.isWindows || Platform.isLinux);

/// GTK owns Linux's decorations, initial geometry and first-frame presentation.
/// Initializing window_manager must not replace its native header bar.
Future<void> initializeDesktopWindow({TargetPlatform? platform}) async {
  if (kIsWeb) return;
  switch (platform ?? defaultTargetPlatform) {
    case TargetPlatform.windows:
      await windowManager.ensureInitialized();
      await windowManager.waitUntilReadyToShow(const WindowOptions(
        titleBarStyle: TitleBarStyle.hidden,
        size: Size(1180, 780),
        minimumSize: Size(430, 660),
        backgroundColor: Colors.transparent,
      ));
    case TargetPlatform.linux:
      await windowManager.ensureInitialized();
    default:
      break;
  }
}

Future<void> restoreDesktopWindow() async {
  await windowManager.show();
  await windowManager.focus();
}

/// Also used by native application-exit requests (for example macOS Cmd+Q).
Future<void> prepareDesktopApplicationExit() async {
  final session = DesktopSession.current;
  if (session == null) return;
  try {
    // Stopping playback is best-effort on explicit exit. A stale endpoint or
    // unresponsive local host must never trap the user in the application.
    await session.shutdown().timeout(const Duration(seconds: 2));
  } catch (error) {
    debugPrint(
        'Playback shutdown unavailable; continuing exit (${error.runtimeType})');
  } finally {
    try {
      await session.dispose().timeout(const Duration(milliseconds: 500));
    } catch (error) {
      debugPrint(
          'Playback connection cleanup incomplete (${error.runtimeType})');
    }
  }
}

/// Attempt to stop shared playback before any platform's process termination path.
/// Both GTK window destruction and Flutter's application exit can race native
/// plugin/engine threads during shutdown. Avoid that teardown after cleanup.
Future<void> quitDesktopApplication(Future<void> Function() releaseResources,
    {TargetPlatform? platform, void Function(int)? exitProcess}) async {
  final target = platform ?? defaultTargetPlatform;
  // Windows deliberately bypasses plugin teardown. Playback shutdown has a
  // bounded opportunity to finish before TerminateProcess ends the GUI.
  await prepareDesktopApplicationExit();
  if (target == TargetPlatform.windows) {
    if (exitProcess != null) {
      exitProcess(0);
      return;
    }
    terminateWindowsProcess();
  }
  try {
    await releaseResources().timeout(const Duration(seconds: 3));
  } catch (error) {
    debugPrint('Desktop resource cleanup failed: $error');
  }
  if (target == TargetPlatform.linux) {
    (exitProcess ?? exit)(0);
    return;
  }
  try {
    await windowManager.destroy();
  } catch (error) {
    debugPrint('Desktop window shutdown failed: $error');
    exit(0);
  }
}
