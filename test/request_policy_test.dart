import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:dio/dio.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:thmr_app/core/network/request_policy.dart';

class _Adapter implements HttpClientAdapter {
  _Adapter(this.reply);
  final Future<ResponseBody> Function(RequestOptions) reply;
  int requests = 0;
  @override
  Future<ResponseBody> fetch(RequestOptions options,
      Stream<Uint8List>? requestStream, Future<void>? cancelFuture) {
    requests++;
    return reply(options);
  }

  @override
  void close({bool force = false}) {}
}

ResponseBody _json([int status = 200, Map<String, List<String>>? headers]) =>
    ResponseBody.fromString(jsonEncode({'ok': true}), status, headers: {
      Headers.contentTypeHeader: ['application/json'],
      ...?headers,
    });

void main() {
  late DateTime now;
  late RequestPolicy policy;
  setUp(() {
    now = DateTime.utc(2026, 11, 9);
    policy = RequestPolicy(
        now: () => now,
        sleep: (delay) async {
          now = now.add(delay);
        });
  });
  ApiHttpClient client(_Adapter adapter) => ApiHttpClient(
      client: Dio(BaseOptions(baseUrl: 'https://local.invalid'))
        ..httpClientAdapter = adapter,
      policy: policy);

  test('100 identical GET callers share one request across API clients',
      () async {
    final gate = Completer<void>();
    final started = Completer<void>();
    final adapter = _Adapter((_) async {
      started.complete();
      await gate.future;
      return _json();
    });
    final first = client(adapter);
    final second = client(adapter);
    final all = Future.wait(List.generate(
        100, (i) => (i.isEven ? first : second).get<Object?>('/status')));
    await started.future;
    expect(adapter.requests, 1);
    gate.complete();
    expect(await all, hasLength(100));
    expect(adapter.requests, 1);
  });

  test('writes are paced and serialized, never deduplicated or replayed',
      () async {
    final starts = <DateTime>[];
    final adapter = _Adapter((_) async {
      starts.add(now);
      return _json();
    });
    final api = client(adapter);
    await Future.wait(List.generate(10, (_) => api.post<Object?>('/vote')));
    expect(adapter.requests, 10);
    for (var i = 1; i < starts.length; i++) {
      expect(starts[i].difference(starts[i - 1]), policy.interval);
    }
  });

  test('queued authorization is captured and separate accounts never coalesce',
      () async {
    final gate = Completer<void>();
    final started = Completer<void>();
    final accounts = <Object?>[];
    final adapter = _Adapter((request) async {
      if (request.path == '/block') {
        started.complete();
        await gate.future;
      } else {
        accounts.add(request.headers['Authorization']);
      }
      return _json();
    });
    final api = client(adapter);
    final blocking = api.get<Object?>('/block');
    await started.future;
    api.options.headers['Authorization'] = 'Bearer account-a';
    final a = api.get<Object?>('/me');
    api.options.headers['Authorization'] = 'Bearer account-b';
    final b = api.get<Object?>('/me');
    gate.complete();
    await Future.wait([blocking, a, b]);
    expect(accounts, ['Bearer account-a', 'Bearer account-b']);
  });

  for (final status in [403, 429, 503]) {
    test('$status stops already queued requests until Retry-After expires',
        () async {
      var blocked = true;
      final adapter = _Adapter((_) async => _json(blocked ? status : 200, {
            'retry-after': ['120']
          }));
      final api = client(adapter);
      final first =
          expectLater(api.get<Object?>('/first'), throwsA(isA<DioException>()));
      final queued = expectLater(
          api.post<Object?>('/write'), throwsA(isA<DioException>()));
      await Future.wait([first, queued]);
      expect(adapter.requests, 1);
      for (var i = 0; i < 100; i++) {
        await expectLater(
            api.get<Object?>('/other'), throwsA(isA<DioException>()));
      }
      expect(adapter.requests, 1);
      now = now.add(const Duration(seconds: 121));
      blocked = false;
      await api.get<Object?>('/recover');
      expect(adapter.requests, 2);
    });
  }

  test('connection resets back off without retrying writes', () async {
    final adapter = _Adapter((request) async => throw DioException(
        requestOptions: request, type: DioExceptionType.connectionError));
    final api = client(adapter);
    await expectLater(
        api.post<Object?>('/write'), throwsA(isA<DioException>()));
    for (var i = 0; i < 20; i++) {
      await expectLater(
          api.post<Object?>('/write'), throwsA(isA<DioException>()));
    }
    expect(adapter.requests, 1);
    now = now.add(const Duration(seconds: 11));
    await expectLater(
        api.post<Object?>('/write'), throwsA(isA<DioException>()));
    now = now.add(const Duration(seconds: 11));
    await expectLater(
        api.post<Object?>('/write'), throwsA(isA<DioException>()));
    expect(adapter.requests, 2, reason: 'Second failure has a longer cooldown');
  });

  test('Retry-After accepts seconds and HTTP dates', () {
    expect(RequestPolicy.retryAfter('120', now), const Duration(seconds: 120));
    expect(
        RequestPolicy.retryAfter(
            HttpDate.format(now.add(const Duration(minutes: 2))), now),
        const Duration(minutes: 2));
    expect(RequestPolicy.retryAfter('bad', now), isNull);
  });

  test('unique request bursts cannot build an unbounded pending queue',
      () async {
    final adapter = _Adapter((_) async => _json());
    final api = client(adapter);
    final results = await Future.wait(List.generate(
        100,
        (i) => api
            .get<Object?>('/unique/$i')
            .then((_) => true, onError: (_) => false)));
    expect(results.where((ok) => ok), hasLength(16));
    expect(adapter.requests, 16);
  });

  test('cancelled queued downloads never reach the network', () async {
    final gate = Completer<void>();
    final started = Completer<void>();
    final adapter = _Adapter((_) async {
      started.complete();
      await gate.future;
      return _json();
    });
    final api = client(adapter);
    final first = api.get<Object?>('/block');
    await started.future;
    final token = CancelToken();
    final cancelled = expectLater(
        api.get<Object?>('/download', cancelToken: token),
        throwsA(isA<DioException>()
            .having((e) => e.type, 'type', DioExceptionType.cancel)));
    token.cancel();
    gate.complete();
    await first;
    await cancelled;
    expect(adapter.requests, 1);
  });
}
