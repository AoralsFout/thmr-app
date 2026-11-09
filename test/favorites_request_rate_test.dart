import 'dart:convert';
import 'dart:io';

import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'package:thmr_app/core/api/radio_api.dart';
import 'package:thmr_app/core/api/thmr_api.dart';
import 'package:thmr_app/core/models/models.dart';
import 'package:thmr_app/core/player/desktop_session.dart';
import 'package:thmr_app/core/player/playback_controller.dart';
import 'package:thmr_app/core/state/app_providers.dart';
import 'package:thmr_app/core/state/auth_provider.dart';
import 'package:thmr_app/core/state/station_provider.dart';

class _CountingApi extends ThmrApi {
  int requests = 0;
  bool limited = false;

  @override
  Future<ThmrUser?> me() async => const ThmrUser(
        id: 'audit',
        naidsId: '',
        username: '',
        nickname: '',
        email: '',
        realnameVerified: false,
        createdAt: null,
        needsNickname: false,
      );

  @override
  Future<FavoritesStatus> favoritesStatus(
      {required String stationId, required String trackId}) async {
    requests++;
    if (limited) throw ApiException(429, 'limited');
    return const FavoritesStatus(
        favorited: false, itemId: null, collectionId: null, collections: []);
  }
}

void main() {
  setUp(() => SharedPreferences.setMockInitialValues({
        'thmr_session_token': 'audit-only-fake-token',
      }));

  for (final limited in [false, true]) {
    test(
        'same-track desktop heartbeats must not query favorites again ($limited)',
        () async {
      final directory = await Directory.systemTemp.createTemp('thmr-audit-');
      final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
      final sockets = <WebSocket>[];
      var volumeCommands = 0;
      final snapshot = <String, dynamic>{
        'protocolVersion': 1,
        'instanceId': 'audit',
        'revision': 1,
        'volume': 70,
        'stationStatus': <String, dynamic>{
          'station': {'id': 'touhou', 'bitrates': []},
          'queue': [],
          'current': {'trackId': 'same-track', 'startedAtMs': 1},
        },
      };
      server.listen((request) async {
        if (request.uri.path == '/v1/events') {
          final socket = await WebSocketTransformer.upgrade(request);
          sockets.add(socket);
          socket.listen((_) {});
          return;
        }
        if (request.method == 'POST') {
          final payload =
              jsonDecode(await utf8.decoder.bind(request).join()) as Map;
          final command = payload['command'] as Map;
          if (command['command'] == 'set_volume') {
            volumeCommands++;
            snapshot['volume'] = command['value'];
            snapshot['revision'] = (snapshot['revision'] as int) + 1;
            for (final socket in sockets) {
              socket.add(jsonEncode(snapshot));
            }
          }
        }
        request.response.headers.contentType = ContentType.json;
        request.response.write(jsonEncode(snapshot));
        await request.response.close();
      });
      final file = File('${directory.path}/instance.json');
      await file.writeAsString(jsonEncode({
        'protocolVersion': 1,
        'instanceId': 'audit',
        'port': server.port,
        'token': 'a' * 64,
      }));
      final session = await DesktopSession.attach(file);
      final playback = PlaybackController.remote(session);
      final api = _CountingApi()..limited = limited;
      final container = ProviderContainer(overrides: [
        authProvider.overrideWith((ref) => AuthController(client: api)),
        stationProvider.overrideWith((ref) => StationPoller.remote(session)),
      ]);
      try {
        while (!container.read(authProvider).initialized) {
          await Future<void>.delayed(Duration.zero);
        }
        container.listen(favoritesStatusProvider, (_, __) {});
        Future<void> settle() async {
          try {
            await container.read(favoritesStatusProvider.future);
          } catch (_) {/* Count requests even when server rejects them. */}
        }

        await settle();
        final initial = api.requests;
        for (var i = 0; i < 60; i++) {
          snapshot['revision'] = i + 2;
          final accepted = session.events.first;
          sockets.single.add(jsonEncode(snapshot));
          await accepted.timeout(const Duration(seconds: 2));
          await settle();
        }
        expect(api.requests, initial,
            reason: 'Heartbeats did not change the track or account');
        expect(container.read(stationProvider).trackGeneration, 1);

        await Future.wait(
            List.generate(100, (i) => playback.setVolume(i.toDouble())));
        await Future<void>.delayed(const Duration(milliseconds: 20));
        await settle();
        expect(api.requests, initial,
            reason: 'Dragging volume must not query the public API');
        expect(volumeCommands, lessThanOrEqualTo(2),
            reason: 'Rapid slider updates should share the latest command');
        expect(playback.state.volume, 99);

        Future<void> publish() async {
          snapshot['revision'] = (snapshot['revision'] as int) + 1;
          final accepted = session.events.first;
          sockets.single.add(jsonEncode(snapshot));
          await accepted.timeout(const Duration(seconds: 2));
          await settle();
        }

        // Playing the same ID in a new round still refreshes favorites.
        final status = snapshot['stationStatus'] as Map<String, dynamic>;
        final current = status['current'] as Map<String, dynamic>;
        current['startedAtMs'] = 2;
        await publish();
        expect(api.requests, initial + 1);

        // Clearing the current track clears the result without an API call.
        status['current'] = null;
        await publish();
        expect(api.requests, initial + 1);
        expect(container.read(favoritesStatusProvider).requireValue.favorited,
            isFalse);

        status['current'] = {...current, 'trackId': 'next-track'};
        await publish();
        expect(api.requests, initial + 2);

        // Mutations can still explicitly refresh the same track.
        container.invalidate(favoritesStatusProvider);
        await settle();
        expect(api.requests, initial + 3);
      } finally {
        container.dispose();
        playback.dispose();
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
