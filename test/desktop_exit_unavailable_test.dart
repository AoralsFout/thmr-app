import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:thmr_app/core/platform/desktop_window.dart';
import 'package:thmr_app/core/player/desktop_session.dart';

class _Host {
  _Host(this.directory, this.server);
  final Directory directory;
  final HttpServer server;
  final sockets = <WebSocket>[];
  final holdShutdown = Completer<void>();
  String mode = 'reject';
  bool listening = true;
  int shutdowns = 0;
  File get file => File('${directory.path}/instance.json');
  final state = <String, dynamic>{
    'protocolVersion': 1,
    'instanceId': 'exit-unavailable',
    'revision': 1,
    'phase': 'online',
  };

  static Future<_Host> create() async {
    final host = _Host(
        await Directory.systemTemp.createTemp('thmr-offline-exit-'),
        await HttpServer.bind(InternetAddress.loopbackIPv4, 0));
    await host.file.writeAsString(jsonEncode({
      'protocolVersion': 1,
      'instanceId': 'exit-unavailable',
      'port': host.server.port,
      'token': 'a' * 64,
      'pid': 1,
    }));
    host.server.listen(host.serve);
    return host;
  }

  Future<void> serve(HttpRequest request) async {
    if (request.uri.path == '/v1/events') {
      final socket = await WebSocketTransformer.upgrade(request);
      sockets.add(socket);
      socket.listen((_) {});
      return;
    }
    try {
      if (request.method == 'POST') {
        final body = jsonDecode(await utf8.decoder.bind(request).join()) as Map;
        expect((body['command'] as Map)['command'], 'shutdown');
        shutdowns++;
        if (mode == 'hang') await holdShutdown.future;
        request.response.statusCode = 503;
        request.response.write(jsonEncode({'error': 'host unavailable'}));
      } else {
        request.response.headers.contentType = ContentType.json;
        request.response.write(jsonEncode(state));
      }
      await request.response.close();
    } on HttpException {
      // The client closes a timed-out shutdown request.
    } on SocketException {
      // The simulated host may be shut down before its response is delivered.
    }
  }

  Future<void> disconnect() async {
    if (listening) {
      listening = false;
      await server.close(force: true);
    }
    for (final socket in sockets) {
      await socket.close();
    }
  }

  Future<void> dispose() async {
    if (!holdShutdown.isCompleted) holdShutdown.complete();
    await disconnect();
    await directory.delete(recursive: true);
  }
}

void main() {
  // Real loopback fixtures reproduce the refused/stalled local host in the UI.
  for (final platform in [TargetPlatform.windows, TargetPlatform.linux]) {
    for (final mode in ['refused', 'reject', 'hang', 'corrupt endpoint']) {
      test('$mode host permits ${platform.name} exit', () async {
        final host = await _Host.create();
        final session = await DesktopSession.attach(host.file);
        DesktopSession.current = session;
        host.mode = mode;
        try {
          if (mode == 'refused') await host.disconnect();
          if (mode == 'corrupt endpoint') await host.file.writeAsString('{');
          var exited = false;
          var released = false;
          await quitDesktopApplication(
              () async {
                released = true;
              },
              platform: platform,
              exitProcess: (_) {
                exited = true;
              }).timeout(const Duration(seconds: 4));
          expect(exited, isTrue);
          expect(released, platform == TargetPlatform.linux);
          await expectLater(session.execute('play'), throwsStateError);
          expect(await host.file.exists(), isTrue,
              reason: 'GUI must not delete a possibly live host endpoint');
        } finally {
          DesktopSession.current = null;
          await session.dispose();
          await host.dispose();
        }
      });
    }
  }

  test('native application exit also tolerates an unavailable host', () async {
    final host = await _Host.create();
    final session = await DesktopSession.attach(host.file);
    DesktopSession.current = session;
    try {
      await host.disconnect();
      await prepareDesktopApplicationExit().timeout(const Duration(seconds: 4));
      await expectLater(session.execute('play'), throwsStateError);
    } finally {
      DesktopSession.current = null;
      await session.dispose();
      await host.dispose();
    }
  });

  test('paused state listeners cannot hold the process open during cleanup',
      () async {
    final host = await _Host.create();
    final session = await DesktopSession.attach(host.file);
    DesktopSession.current = session;
    final paused = session.events.listen((_) {})..pause();
    try {
      var exited = false;
      await quitDesktopApplication(() async {},
          platform: TargetPlatform.windows, exitProcess: (_) {
        exited = true;
      }).timeout(const Duration(seconds: 2));
      expect(exited, isTrue);
    } finally {
      DesktopSession.current = null;
      paused.resume();
      await paused.cancel();
      await session.dispose();
      await host.dispose();
    }
  });
}
