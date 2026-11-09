import 'dart:async';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:thmr_app/core/player/shared_radio_stream_io.dart';

void main() {
  test('stream reconnect attempts respect upstream 429 without reopening it',
      () async {
    final upstream = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
    final relay = SharedRadioStream();
    final player = HttpClient();
    var requests = 0;
    String? agent;
    upstream.listen((request) async {
      requests++;
      agent = request.headers.value(HttpHeaders.userAgentHeader);
      request.response.statusCode = 429;
      request.response.headers.set('retry-after', '120');
      await request.response.close();
    });
    try {
      for (var i = 0; i < 5; i++) {
        final uri =
            await relay.activate('http://127.0.0.1:${upstream.port}/live.mp3');
        final response = await (await player.getUrl(uri)).close();
        await response.drain<void>().timeout(const Duration(seconds: 2));
      }
      expect(requests, 1);
      expect(agent, startsWith('THMR-Client/'));
    } finally {
      await relay.dispose();
      player.close(force: true);
      await upstream.close(force: true);
    }
  });

  test('player and spectrum share one upstream radio request', () async {
    final upstream = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
    final relay = SharedRadioStream();
    final playerClient = HttpClient();
    final spectrumClient = HttpClient();
    addTearDown(() async {
      await relay.dispose();
      playerClient.close(force: true);
      spectrumClient.close(force: true);
      await upstream.close(force: true);
    });

    var upstreamRequests = 0;
    final releaseLastChunk = Completer<void>();
    upstream.listen((request) async {
      upstreamRequests++;
      request.response.headers.contentType = ContentType('audio', 'mpeg');
      request.response.bufferOutput = false;
      request.response.add(List<int>.filled(1024, 0x41));
      await request.response.flush();
      await releaseLastChunk.future;
      request.response.add(List<int>.filled(1024, 0x42));
      await request.response.close();
    });

    final playUri =
        await relay.activate('http://127.0.0.1:${upstream.port}/live.mp3');
    final playRequest = await playerClient.getUrl(playUri);
    final playResponse = await playRequest.close();
    final firstChunk = Completer<void>();
    final playDone = Completer<List<int>>();
    final playBytes = <int>[];
    playResponse.listen(
      (chunk) {
        playBytes.addAll(chunk);
        if (!firstChunk.isCompleted) firstChunk.complete();
      },
      onDone: () => playDone.complete(playBytes),
      onError: playDone.completeError,
    );
    await firstChunk.future.timeout(const Duration(seconds: 5));

    final spectrumRequest = await spectrumClient.getUrl(relay.spectrumUri!);
    final spectrumResponse = await spectrumRequest.close();
    final spectrumDone =
        spectrumResponse.fold<List<int>>(<int>[], (bytes, part) {
      bytes.addAll(part);
      return bytes;
    });
    releaseLastChunk.complete();

    final results = await Future.wait([playDone.future, spectrumDone])
        .timeout(const Duration(seconds: 5));
    expect(upstreamRequests, 1);
    expect(results[0], [
      ...List<int>.filled(1024, 0x41),
      ...List<int>.filled(1024, 0x42),
    ]);
    expect(results[1], results[0]);
  });
}
