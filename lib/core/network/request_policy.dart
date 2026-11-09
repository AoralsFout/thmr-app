import 'dart:async';
import 'dart:io' show HttpDate, HttpException;
import 'dart:math' as math;

import 'package:dio/dio.dart';

/// No retries: writes must never be replayed. A host shares one paced queue
/// across API clients; a rejection also stops requests already waiting in it.
class RequestPolicy {
  RequestPolicy({
    this.interval = const Duration(milliseconds: 500),
    DateTime Function()? now,
    Future<void> Function(Duration)? sleep,
  })  : now = now ?? DateTime.now,
        _sleep = sleep ?? Future<void>.delayed;

  static final shared = RequestPolicy();
  final Duration interval;
  final DateTime Function() now;
  final Future<void> Function(Duration) _sleep;
  final _hosts = <String, _HostRequests>{};
  final _reads = <String, Future<Object?>>{};

  static Duration? retryAfter(String? value, DateTime now) {
    if (value == null) return null;
    final seconds = int.tryParse(value.trim());
    if (seconds != null) return Duration(seconds: math.max(0, seconds));
    try {
      final duration = HttpDate.parse(value).difference(now);
      return duration.isNegative ? Duration.zero : duration;
    } on HttpException {
      return null;
    }
  }

  Future<T> run<T>(Uri uri, Future<T> Function() send, {String? readKey}) {
    final key = readKey == null ? null : '${uri.origin}|$readKey';
    final pending = key == null ? null : _reads[key];
    if (pending != null) return pending.then((value) => value as T);
    final host = _hosts.putIfAbsent(uri.origin, _HostRequests.new);
    if (host.queued >= 16) {
      return Future.error(RequestCoolingDown(now().add(interval), null));
    }
    final previous = host.tail;
    final released = Completer<void>();
    host.tail = released.future;
    host.queued++;
    final future = () async {
      try {
        await previous;
        void checkCooldown() {
          final until = host.blockedUntil;
          if (until != null && now().isBefore(until)) {
            throw RequestCoolingDown(until, host.status);
          }
        }

        checkCooldown();
        final last = host.startedAt;
        if (last != null) {
          final wait = interval - now().difference(last);
          if (wait > Duration.zero) await _sleep(wait);
        }
        checkCooldown();
        host.startedAt = now();
        return await send();
      } finally {
        host.queued--;
        if (key != null) _reads.remove(key);
        released.complete();
      }
    }();
    if (key != null) _reads[key] = future;
    return future;
  }

  void response(Uri uri, int? status,
      {String? retryAfterHeader, bool connectionFailed = false}) {
    final host = _hosts.putIfAbsent(uri.origin, _HostRequests.new);
    if (status == 403 ||
        status == 429 ||
        (status ?? 0) >= 500 ||
        connectionFailed) {
      host.failures = math.min(host.failures + 1, 8);
      final seconds = math.min(300, 10 * (1 << (host.failures - 1)));
      var delay = Duration(seconds: seconds);
      if ((status == 403 || status == 429) &&
          delay < const Duration(minutes: 1)) {
        delay = const Duration(minutes: 1);
      }
      final requested = retryAfter(retryAfterHeader, now());
      if (requested != null && requested > delay) delay = requested;
      final until = now().add(delay);
      if (host.blockedUntil == null || until.isAfter(host.blockedUntil!)) {
        host.blockedUntil = until;
      }
      host.status = status;
    } else if (status != null && status >= 200 && status < 400) {
      host.failures = 0;
      host.blockedUntil = null;
      host.status = null;
    }
  }
}

class _HostRequests {
  Future<void> tail = Future.value();
  DateTime? startedAt;
  DateTime? blockedUntil;
  int? status;
  int queued = 0;
  int failures = 0;
}

class RequestCoolingDown implements Exception {
  const RequestCoolingDown(this.until, this.status);
  final DateTime until;
  final int? status;
  @override
  String toString() => '请求暂缓，请稍后重试';
}

/// Account, radio and update requests use the same protection. Authentication
/// and response shape are part of GET identity; writes are never coalesced.
class ApiHttpClient {
  ApiHttpClient({Dio? client, RequestPolicy? policy})
      : _client = client ?? Dio(),
        policy = policy ?? RequestPolicy.shared {
    options.headers.putIfAbsent(
        'User-Agent', () => 'THMR-Client/1.0 (+https://thmr.yunmoan.cn)');
  }
  final Dio _client;
  final RequestPolicy policy;
  BaseOptions get options => _client.options;
  void close({bool force = false}) => _client.close(force: force);

  Future<Response<T>> _request<T>(
    String method,
    String path, {
    Object? data,
    Map<String, dynamic>? queryParameters,
    Options? options,
    CancelToken? cancelToken,
  }) async {
    final settings = (options ?? Options()).copyWith(method: method);
    final request = settings.compose(_client.options, path,
        data: data, queryParameters: queryParameters, cancelToken: cancelToken);
    final wireOptions =
        settings.copyWith(headers: Map<String, dynamic>.from(request.headers));
    final headers = request.headers.entries.toList()
      ..sort((a, b) => a.key.compareTo(b.key));
    final readKey = method == 'GET' &&
            cancelToken == null &&
            request.responseType != ResponseType.stream
        ? '$T|${request.uri}|${request.responseType}|${headers.map((e) => '${e.key}:${e.value}').join('\n')}'
        : null;
    try {
      return await policy.run(request.uri, () async {
        if (cancelToken?.isCancelled == true) throw cancelToken!.cancelError!;
        try {
          final response = await _client.request<T>(path,
              data: data,
              queryParameters: queryParameters,
              options: wireOptions,
              cancelToken: cancelToken);
          policy.response(request.uri, response.statusCode,
              retryAfterHeader: response.headers.value('retry-after'));
          return response;
        } on DioException catch (error) {
          if (!CancelToken.isCancel(error)) {
            policy.response(request.uri, error.response?.statusCode,
                retryAfterHeader: error.response?.headers.value('retry-after'),
                connectionFailed: {
                  DioExceptionType.connectionError,
                  DioExceptionType.connectionTimeout,
                  DioExceptionType.receiveTimeout,
                  DioExceptionType.sendTimeout,
                }.contains(error.type));
          }
          rethrow;
        }
      }, readKey: readKey);
    } on RequestCoolingDown catch (error) {
      throw DioException(
          requestOptions: request,
          type: error.status == null
              ? DioExceptionType.connectionError
              : DioExceptionType.badResponse,
          response: error.status == null
              ? null
              : Response(
                  requestOptions: request,
                  statusCode: error.status,
                  headers: Headers.fromMap({
                    'retry-after': [
                      '${math.max(1, error.until.difference(policy.now()).inSeconds)}'
                    ],
                  })),
          error: error,
          message: error.toString());
    }
  }

  Future<Response<T>> get<T>(String path,
          {Map<String, dynamic>? queryParameters,
          Options? options,
          CancelToken? cancelToken}) =>
      _request('GET', path,
          queryParameters: queryParameters,
          options: options,
          cancelToken: cancelToken);
  Future<Response<T>> post<T>(String path,
          {Object? data,
          Map<String, dynamic>? queryParameters,
          Options? options}) =>
      _request('POST', path,
          data: data, queryParameters: queryParameters, options: options);
  Future<Response<T>> patch<T>(String path, {Object? data}) =>
      _request('PATCH', path, data: data);
  Future<Response<T>> delete<T>(String path,
          {Object? data, Map<String, dynamic>? queryParameters}) =>
      _request('DELETE', path, data: data, queryParameters: queryParameters);
}
