import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:thmr_app/core/player/desktop_session.dart';
import 'package:thmr_app/core/player/playback_controller.dart';
import 'package:thmr_app/core/state/station_provider.dart';

void main() {
  test('desktop adapters share confirmed state without creating a local player',
      () async {
    final directory =
        await Directory.systemTemp.createTemp('thmr-session-test-');
    final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
    const token =
        'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';
    var revision = 1;
    final sockets = <WebSocket>[];
    final commands = <String>[];
    final state = <String, dynamic>{
      'protocolVersion': 1,
      'instanceId': 'test',
      'revision': revision,
      'phase': 'idle',
      'userPaused': true,
      'volume': 35,
      'lastAudibleVolume': 35,
      'listenedSec': 12,
      'source': 'source1',
      'radioBaseUrl': 'http://127.0.0.1',
      'stationId': 'touhou',
      'profileId': '128k',
      'stationStatus': {
        'station': {'id': 'touhou', 'bitrates': []},
        'totals': {},
        'queue': [],
        'current': {
          'trackId': '1',
          'title': '测试',
          'artist': 'THMR',
          'startedAtMs': 1
        },
      },
    };
    void publish() {
      for (final socket in sockets) {
        socket.add(jsonEncode(state));
      }
    }

    server.listen((request) async {
      expect(request.headers.value(HttpHeaders.authorizationHeader),
          'Bearer $token');
      if (request.uri.path == '/v1/events') {
        expect(request.headers.value('X-THMR-Protocol'), '1');
        final socket = await WebSocketTransformer.upgrade(request);
        socket.listen((_) {});
        sockets.add(socket);
        socket.add(jsonEncode(state));
        return;
      }
      if (request.method == 'POST') {
        final body =
            (jsonDecode(await utf8.decoder.bind(request).join()) as Map);
        final command = (body['command'] as Map).cast<String, dynamic>();
        commands.add(command['command'] as String);
        state['revision'] = ++revision;
        if (command['command'] == 'play') {
          state['phase'] = 'online';
          state['userPaused'] = false;
        }
        if (command['command'] == 'set_volume') {
          state['volume'] = command['value'];
        }
        publish();
      }
      request.response.headers.contentType = ContentType.json;
      request.response.write(jsonEncode(state));
      await request.response.close();
    });
    final file = File('${directory.path}/instance.json');
    await file.writeAsString(jsonEncode({
      'protocolVersion': 1,
      'port': server.port,
      'token': token,
      'instanceId': 'test',
      'pid': 1
    }));
    final session = await DesktopSession.attach(file);
    final playback = PlaybackController.remote(session);
    final station = StationPoller.remote(session);
    try {
      expect(playback.state.volume, 35);
      expect(playback.state.userPaused, isTrue);
      expect(station.state.status?.current?.title, '测试');
      await playback.playFromSystem();
      await playback.setVolume(42);
      await Future<void>.delayed(Duration.zero);
      expect(commands, ['play', 'set_volume']);
      expect(playback.state.phase, PlaybackPhase.online);
      expect(playback.state.volume, 42);
      // A delayed response/event cannot replace a newer confirmed revision.
      for (final socket in sockets) {
        socket.add(jsonEncode({...state, 'revision': 0, 'volume': 1}));
      }
      await Future<void>.delayed(const Duration(milliseconds: 30));
      expect(playback.state.volume, 42);
      expect(station.state.trackGeneration, 1);
    } finally {
      playback.dispose();
      station.dispose();
      await session.dispose();
      for (final socket in sockets) {
        await socket.close();
      }
      await server.close(force: true);
      await directory.delete(recursive: true);
    }
  });

  test('incompatible desktop endpoint fails before any connection is made',
      () async {
    final directory =
        await Directory.systemTemp.createTemp('thmr-version-test-');
    try {
      final file = File('${directory.path}/instance.json');
      await file.writeAsString(
          jsonEncode({'protocolVersion': 999, 'port': 1, 'token': 'invalid'}));
      await expectLater(DesktopSession.attach(file), throwsStateError);
    } finally {
      await directory.delete(recursive: true);
    }
  });
}
