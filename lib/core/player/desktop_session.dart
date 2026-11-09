import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:path/path.dart' as p;

/// One desktop playback owner shared by the GUI, CLI, TUI and native controls.
/// The endpoint credential stays in memory and is never exposed to providers.
class DesktopSession {
  DesktopSession._(this._instanceFile);

  static const protocolVersion = 1;
  static DesktopSession? current;

  final File _instanceFile;
  final HttpClient _http = HttpClient()
    ..connectionTimeout = const Duration(seconds: 4)
    ..findProxy = ((_) => 'DIRECT');
  final _events = StreamController<Map<String, dynamic>>.broadcast();
  final _errors = StreamController<Object>.broadcast();
  Map<String, dynamic> snapshot = {};
  Stream<Map<String, dynamic>> get events => _events.stream;
  Stream<Object> get errors => _errors.stream;
  WebSocket? _socket;
  Timer? _retry;
  bool _disposed = false;
  int _epoch = 0;

  static Future<DesktopSession> open() async {
    final sibling = p.join(p.dirname(Platform.resolvedExecutable),
        Platform.isWindows ? 'thmr.exe' : 'thmr');
    final executable = Platform.environment['THMR_CLI_PATH'] ?? sibling;
    if (!await File(executable).exists()) {
      throw const FileSystemException('未找到播放程序，请安装完整 THMR 客户端。');
    }
    final paths = await Process.run(executable, ['paths', '--json'])
        .timeout(const Duration(seconds: 8));
    if (paths.exitCode != 0) throw StateError('无法读取播放设置目录：${paths.stderr}');
    final directories =
        (jsonDecode(paths.stdout as String) as Map).cast<String, dynamic>();
    final started = await Process.run(executable, ['daemon', 'start'])
        .timeout(const Duration(seconds: 18));
    if (started.exitCode != 0) {
      throw StateError('无法启动播放：${started.stderr}');
    }
    return attach(
        File(p.join(directories['runtime'] as String, 'instance.json')));
  }

  /// Also used by integration tests against an isolated host.
  static Future<DesktopSession> attach(File instanceFile) async {
    final session = DesktopSession._(instanceFile);
    try {
      await session._connect();
      return session;
    } catch (_) {
      await session.dispose();
      rethrow;
    }
  }

  Future<Map<String, dynamic>> _endpoint() async {
    final endpoint = (jsonDecode(await _instanceFile.readAsString()) as Map)
        .cast<String, dynamic>();
    if (endpoint['protocolVersion'] != protocolVersion) {
      throw StateError('播放程序版本不兼容，请更新完整客户端。');
    }
    final port = endpoint['port'];
    final token = endpoint['token'];
    if (port is! int ||
        port < 1 ||
        port > 65535 ||
        token is! String ||
        !RegExp(r'^[0-9a-fA-F]{64}$').hasMatch(token)) {
      throw const FormatException('播放连接信息无效');
    }
    return endpoint;
  }

  Future<Map<String, dynamic>> _request(
      Map<String, dynamic> endpoint, Map<String, dynamic>? command) async {
    final uri = Uri(
        scheme: 'http',
        host: '127.0.0.1',
        port: endpoint['port'] as int,
        path: command == null ? '/v1/state' : '/v1/command');
    final request = await _http
        .openUrl(command == null ? 'GET' : 'POST', uri)
        .timeout(const Duration(seconds: 5));
    request.headers
        .set(HttpHeaders.authorizationHeader, 'Bearer ${endpoint['token']}');
    request.headers.set('X-THMR-Protocol', '$protocolVersion');
    if (command != null) {
      request.headers.contentType = ContentType.json;
      request.write(
          jsonEncode({'protocolVersion': protocolVersion, 'command': command}));
    }
    final response = await request.close().timeout(const Duration(seconds: 5));
    final bytes = <int>[];
    await for (final chunk in response.timeout(const Duration(seconds: 5))) {
      bytes.addAll(chunk);
      if (bytes.length > 1024 * 1024) throw const FormatException('播放状态过大');
    }
    final data =
        (jsonDecode(utf8.decode(bytes)) as Map).cast<String, dynamic>();
    if (response.statusCode != 200) {
      throw StateError(data['error'] as String? ?? '播放请求失败');
    }
    if (data['protocolVersion'] != protocolVersion ||
        data['instanceId'] != endpoint['instanceId']) {
      throw StateError('播放连接已变更，请重新连接。');
    }
    return data;
  }

  void _accept(Map<String, dynamic> state) {
    if (_disposed) return;
    if (snapshot['instanceId'] == state['instanceId'] &&
        (snapshot['revision'] as int? ?? -1) >
            (state['revision'] as int? ?? -1)) {
      return;
    }
    snapshot = state;
    _events.add(state);
  }

  Future<Map<String, dynamic>> execute(String command,
      [Map<String, dynamic> fields = const {}]) async {
    // No automatic retries: toggle must never be executed twice.
    final result =
        await _request(await _endpoint(), {'command': command, ...fields});
    _accept(result);
    return result;
  }

  Future<void> _connect() async {
    final epoch = ++_epoch;
    final endpoint = await _endpoint();
    _accept(await _request(endpoint, null));
    final socket = await WebSocket.connect(
      'ws://127.0.0.1:${endpoint['port']}/v1/events',
      headers: {
        'Authorization': 'Bearer ${endpoint['token']}',
        'X-THMR-Protocol': '$protocolVersion'
      },
      customClient: _http,
    ).timeout(const Duration(seconds: 5));
    if (_disposed || epoch != _epoch) {
      await socket.close();
      return;
    }
    final previous = _socket;
    _socket = socket;
    await previous?.close();
    socket.pingInterval = const Duration(seconds: 15);
    socket.listen((dynamic message) {
      if (_disposed || epoch != _epoch) return;
      try {
        final state =
            (jsonDecode(message as String) as Map).cast<String, dynamic>();
        if (state['protocolVersion'] != protocolVersion ||
            state['instanceId'] != endpoint['instanceId']) {
          throw const FormatException('播放状态版本不兼容');
        }
        _accept(state);
      } catch (error) {
        _reconnect(epoch, error);
      }
    },
        onError: (Object error) => _reconnect(epoch, error),
        onDone: () => _reconnect(epoch, StateError('播放连接已断开')));
  }

  void _reconnect(int epoch, Object error) {
    if (_disposed || epoch != _epoch || _retry?.isActive == true) return;
    _errors.add(error);
    _retry = Timer(const Duration(seconds: 1), () async {
      try {
        await _connect();
      } catch (error) {
        _reconnect(_epoch, error);
      }
    });
  }

  Future<void> dispose() async {
    if (_disposed) return;
    _disposed = true;
    ++_epoch;
    _retry?.cancel();
    await _socket?.close();
    _http.close(force: true);
    await _events.close();
    await _errors.close();
  }
}
