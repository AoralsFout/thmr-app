import 'dart:convert';
import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:thmr_app/core/platform/desktop_window.dart';
import 'package:thmr_app/core/player/desktop_session.dart';

void main() {
  // No widget binding: use real loopback HTTP instead of its HTTP mock.
  for (final scenario in ['playing', 'already stopped', 'shutdown fails']) {
    test('GUI exit stops shared playback before Windows exit: $scenario',
        () async {
      final directory =
          await Directory.systemTemp.createTemp('thmr-exit-test-');
      final file = File('${directory.path}/instance.json');
      final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
      const token =
          'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';
      final sockets = <WebSocket>[];
      final commands = <String>[];
      var playing = true;
      var rejectShutdown = scenario == 'shutdown fails';
      final state = <String, dynamic>{
        'protocolVersion': 1,
        'instanceId': 'exit-test',
        'revision': 1,
        'phase': 'online',
      };
      server.listen((request) async {
        expect(request.headers.value(HttpHeaders.authorizationHeader),
            'Bearer $token');
        if (request.uri.path == '/v1/events') {
          final socket = await WebSocketTransformer.upgrade(request);
          socket.listen((_) {});
          sockets.add(socket);
          socket.add(jsonEncode(state));
          return;
        }
        if (request.method == 'POST') {
          final body =
              jsonDecode(await utf8.decoder.bind(request).join()) as Map;
          final command = (body['command'] as Map)['command'] as String;
          commands.add(command);
          if (rejectShutdown) {
            request.response.statusCode = HttpStatus.serviceUnavailable;
            request.response
                .write(jsonEncode({'error': 'test shutdown failure'}));
            await request.response.close();
            return;
          }
          if (command == 'shutdown') {
            playing = false;
            state['phase'] = 'idle';
            state['revision'] = 2;
          }
        }
        request.response.headers.contentType = ContentType.json;
        request.response.write(jsonEncode(state));
        await request.response.close();
        if (!playing) {
          await Future<void>.delayed(const Duration(milliseconds: 30));
          for (final socket in sockets) {
            await socket.close();
          }
          if (await file.exists()) await file.delete();
        }
      });
      await file.writeAsString(jsonEncode({
        'protocolVersion': 1,
        'instanceId': 'exit-test',
        'port': server.port,
        'token': token,
        'pid': 1,
      }));
      final session = await DesktopSession.attach(file);
      DesktopSession.current = session;
      try {
        var exited = false;
        Future<void> quit() => quitDesktopApplication(() async {},
                platform: TargetPlatform.windows, exitProcess: (_) {
              expect(playing, isFalse,
                  reason: 'GUI exited while music was playing');
              expect(file.existsSync(), isFalse,
                  reason: 'daemon still owns its endpoint');
              exited = true;
            });
        if (scenario == 'already stopped') {
          playing = false;
          for (final socket in sockets) {
            await socket.close();
          }
          await file.delete();
        }
        if (rejectShutdown) {
          // Strict session shutdown still reports failure to callers that
          // require confirmation; explicit GUI exit is tested separately.
          await expectLater(session.shutdown(), throwsStateError);
          expect(exited, isFalse);
          expect(playing, isTrue);
          rejectShutdown = false;
        }
        final closing = quit();
        await expectLater(session.execute('play'), throwsStateError);
        await closing;
        await quit(); // Repeated/native exit notifications are harmless.
        expect(
            commands,
            scenario == 'already stopped'
                ? <String>[]
                : scenario == 'shutdown fails'
                    ? ['shutdown', 'shutdown']
                    : ['shutdown']);
        expect(exited, isTrue);
      } finally {
        DesktopSession.current = null;
        await session.dispose();
        for (final socket in sockets) {
          await socket.close();
        }
        await server.close(force: true);
        await directory.delete(recursive: true);
      }
    });
  }
}
