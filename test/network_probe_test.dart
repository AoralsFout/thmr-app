import 'dart:async';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:thmr_app/core/network/network_probe.dart';

void main() {
  test('probe cooldown is shared across instances and does not retry 429',
      () async {
    final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
    var requests = 0;
    server.listen((request) async {
      requests++;
      request.response.statusCode = 429;
      request.response.headers.set('retry-after', '120');
      await request.response.close();
    });
    final first = NetworkProbe();
    final second = NetworkProbe();
    try {
      final uri = Uri.parse('http://127.0.0.1:${server.port}/api');
      expect((await first.get(uri, maxBytes: 0)).statusCode, 429);
      for (var i = 0; i < 20; i++) {
        final blocked = await second.get(uri, maxBytes: 0);
        expect(blocked.error, contains('冷却'));
        expect(blocked.statusCode, isNull, reason: 'No HTTP request was sent');
      }
      expect(requests, 1);
    } finally {
      first.close();
      second.close();
      await server.close(force: true);
    }
  });
  test('records the connected peer and only public diagnostic headers',
      () async {
    final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
    final probe = NetworkProbe();
    addTearDown(() async {
      probe.close();
      await server.close(force: true);
    });
    server.listen((request) async {
      request.response.headers.set('EO-Cache-Status', 'HIT');
      request.response.headers.set('EO-LOG-UUID', 'request-fixture');
      request.response.headers.set('set-cookie', 'session=private-cookie');
      request.response.headers.set('authorization', 'Bearer private-token');
      request.response.write('private response body');
      await request.response.close();
    });
    final result = await probe.get(
        Uri.parse(
            'http://127.0.0.1:${server.port}/service?token=private-query'),
        maxBytes: 0);
    expect(result.successful, isTrue);
    expect(result.remoteAddress, '127.0.0.1');
    expect(result.remotePort, server.port);
    expect(result.headers['eo-cache-status'], 'HIT');
    expect(result.headers['eo-log-uuid'], 'request-fixture');
    expect(result.headers.keys, isNot(contains('set-cookie')));
    expect(result.headers.keys, isNot(contains('authorization')));
    expect(result.body, isNull);
    expect(result.uri.toString(), isNot(contains('private-query')));
  });

  test('live audio sampling stops after a bounded number of bytes', () async {
    final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
    final probe = NetworkProbe();
    final release = Completer<void>();
    addTearDown(() async {
      release.complete();
      probe.close();
      await server.close(force: true);
    });
    server.listen((request) async {
      try {
        request.response.headers.contentType = ContentType('audio', 'mpeg');
        request.response.add(List.filled(65536, 0x55));
        await request.response.flush();
        await release.future;
        await request.response.close();
      } catch (_) {/* The sampler deliberately closes an ongoing stream. */}
    });
    final result = await probe.get(
        Uri.parse('http://127.0.0.1:${server.port}/live'),
        maxBytes: 32768,
        captureBody: true,
        timeout: const Duration(seconds: 2));
    expect(result.successful, isTrue);
    expect(result.audio, isTrue);
    expect(result.bytesRead, 32768);
    expect(result.body, hasLength(32768));
  });

  test('a hung service times out without failing another request', () async {
    final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
    final independent = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
    final probe = NetworkProbe();
    final release = Completer<void>();
    addTearDown(() async {
      release.complete();
      probe.close();
      await server.close(force: true);
      await independent.close(force: true);
    });
    server.listen((request) async {
      if (request.uri.path == '/hung') await release.future;
      try {
        await request.response.close();
      } catch (_) {}
    });
    independent.listen((request) async {
      await request.response.close();
    });
    final results = await Future.wait([
      probe.get(Uri.parse('http://127.0.0.1:${server.port}/hung'),
          timeout: const Duration(milliseconds: 100)),
      probe.get(Uri.parse('http://127.0.0.1:${independent.port}/ok'),
          maxBytes: 0),
    ]);
    expect(results[0].error, contains('超时'));
    expect(results[1].successful, isTrue);
  });

  test('HTTP 404 still proves that a service is connected', () async {
    final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
    final probe = NetworkProbe();
    addTearDown(() async {
      probe.close();
      await server.close(force: true);
    });
    server.listen((request) async {
      request.response.statusCode = 404;
      await request.response.close();
    });
    final result = await probe.get(Uri.parse('http://127.0.0.1:${server.port}'),
        maxBytes: 0);
    expect(result.reachable, isTrue);
    expect(result.statusLabel, '可连接 · HTTP 404');
    expect(result.successful, isFalse);
  });

  test('HTTP access restrictions are distinguished from network failure', () {
    final forbidden = NetworkProbeResult(
        uri: Uri.parse('https://example.com'), statusCode: 403);
    expect(forbidden.reachable, isTrue);
    expect(forbidden.successful, isFalse);
    expect(forbidden.statusLabel, contains('访问受限'));
    expect(
        publicUrl(Uri.parse(
            'https://user:secret@example.com/path?token=secret#secret')),
        'https://example.com/path');
  });
}
