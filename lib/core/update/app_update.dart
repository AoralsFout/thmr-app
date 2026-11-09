import 'dart:convert';
import 'dart:io' show File;

import 'package:dio/dio.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:pub_semver/pub_semver.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../platform/client_version.dart';
import '../network/request_policy.dart';
import 'update_manifest.dart';
import 'update_packages.dart';

export '../platform/client_version.dart';
export 'update_manifest.dart';

class AppUpdateService {
  AppUpdateService(
      {Dio? client,
      RequestPolicy? policy,
      this.apiUrl = updateApiUrl,
      this.publicKey = updatePublicKey})
      : _client = ApiHttpClient(
            policy: policy,
            client: client ??
                Dio(BaseOptions(
                  connectTimeout: const Duration(seconds: 10),
                  receiveTimeout: const Duration(seconds: 20),
                )));
  final ApiHttpClient _client;
  final String apiUrl;
  final String publicKey;
  void close() => _client.close(force: true);
  Future<AppRelease?> parseRelease(Map<String, dynamic> data) =>
      verifyUpdateManifest(data, publicKey);

  Future<AppRelease?> latestRelease() async {
    checkedUpdateUrl(apiUrl);
    try {
      final response = await _client.get<Object?>(apiUrl,
          options: Options(headers: {
            'Accept': 'application/json',
            'User-Agent': 'THMR-App'
          }));
      if (response.statusCode == 204) return null;
      return await verifyUpdateManifest(response.data, publicKey);
    } on DioException catch (error) {
      final status = error.response?.statusCode;
      final cooling = error.error;
      final retry = cooling is RequestCoolingDown
          ? cooling.until.difference(_client.policy.now())
          : RequestPolicy.retryAfter(
              error.response?.headers.value('retry-after'),
              _client.policy.now());
      if (status == 403 || status == 429) {
        throw UpdateCheckException('更新服务暂时限流或拒绝访问，请稍后重试',
            retryAfter: retry ?? const Duration(minutes: 1));
      }
      throw UpdateCheckException('无法连接更新 API，请检查网络与服务部署后重试', retryAfter: retry);
    }
  }
}

enum UpdateStatus {
  idle,
  checking,
  available,
  current,
  noRelease,
  failed,
  downloading,
  ready,
  installing
}

class AppUpdateState {
  const AppUpdateState(
      {this.status = UpdateStatus.idle,
      this.autoCheck = true,
      this.autoDownload = true,
      this.release,
      this.lastChecked,
      this.ignoredVersion,
      this.error,
      this.progress = 0,
      this.installNotice});
  final UpdateStatus status;
  final bool autoCheck;
  final bool autoDownload;
  final AppRelease? release;
  final DateTime? lastChecked;
  final String? ignoredVersion;
  final String? error;
  final String? installNotice;
  final double progress;
  bool get busy => [
        UpdateStatus.checking,
        UpdateStatus.downloading,
        UpdateStatus.installing
      ].contains(status);
  bool get showNotice =>
      release != null &&
      release!.identity != ignoredVersion &&
      ![UpdateStatus.checking, UpdateStatus.current, UpdateStatus.noRelease]
          .contains(status);

  AppUpdateState copyWith(
          {UpdateStatus? status,
          bool? autoCheck,
          bool? autoDownload,
          AppRelease? release,
          DateTime? lastChecked,
          String? ignoredVersion,
          String? error,
          double? progress,
          String? installNotice}) =>
      AppUpdateState(
          status: status ?? this.status,
          autoCheck: autoCheck ?? this.autoCheck,
          autoDownload: autoDownload ?? this.autoDownload,
          release: release ?? this.release,
          lastChecked: lastChecked ?? this.lastChecked,
          ignoredVersion: ignoredVersion ?? this.ignoredVersion,
          error: error,
          progress: progress ?? this.progress,
          installNotice: installNotice ?? this.installNotice);
}

class AppUpdateController extends StateNotifier<AppUpdateState> {
  AppUpdateController(
      {required AppUpdateService service,
      required Future<ClientVersion> Function() installedVersion,
      UpdatePackageManager? packages,
      DateTime Function()? now})
      : _service = service,
        _packages = packages,
        _installedVersion = installedVersion,
        _now = now ?? DateTime.now,
        super(const AppUpdateState()) {
    ready = _load();
  }

  static const checkInterval = Duration(hours: 24);
  static const _autoKey = 'appUpdate.autoCheck';
  static const _downloadKey = 'appUpdate.autoDownload';
  static const _checkedKey = 'appUpdate.lastChecked';
  static const _ignoredKey = 'appUpdate.ignoredVersion';
  static const _releaseKey = 'appUpdate.release';
  static const _attemptKey = 'appUpdate.lastAttempt';
  static const _retryKey = 'appUpdate.retryNotBefore';
  final AppUpdateService _service;
  final UpdatePackageManager? _packages;
  final Future<ClientVersion> Function() _installedVersion;
  final DateTime Function() _now;
  late final Future<void> ready;
  SharedPreferences? _prefs;
  DateTime? _lastAttempt;
  DateTime? _retryNotBefore;
  File? _downloaded;
  int _downloadGeneration = 0;

  bool _newer(AppRelease release, ClientVersion installed) {
    final current = Version.parse(installed.version);
    return release.version > current ||
        (release.version == current &&
            release.buildNumber > (int.tryParse(installed.buildNumber) ?? 0));
  }

  Future<void> _load() async {
    try {
      _prefs = await SharedPreferences.getInstance();
      if (!mounted) return;
      _lastAttempt = DateTime.tryParse(_prefs!.getString(_attemptKey) ?? '');
      _retryNotBefore = DateTime.tryParse(_prefs!.getString(_retryKey) ?? '');
      state = state.copyWith(
          autoCheck: _prefs!.getBool(_autoKey) ?? true,
          autoDownload: _prefs!.getBool(_downloadKey) ?? true,
          lastChecked: DateTime.tryParse(_prefs!.getString(_checkedKey) ?? ''),
          ignoredVersion: _prefs!.getString(_ignoredKey));
      final raw = _prefs!.getString(_releaseKey);
      if (raw != null) {
        try {
          final release = await _service
              .parseRelease((jsonDecode(raw) as Map).cast<String, dynamic>());
          final installed = await _installedVersion();
          if (!mounted) return;
          if (release != null && _newer(release, installed)) {
            state = state.copyWith(
                status: UpdateStatus.available, release: release);
          }
        } catch (_) {
          await _prefs!.remove(_releaseKey);
          await _prefs!.remove(_checkedKey);
          if (mounted) {
            state = AppUpdateState(
                autoCheck: state.autoCheck,
                autoDownload: state.autoDownload,
                ignoredVersion: state.ignoredVersion);
          }
        }
      }
      final notice = await _packages?.previousInstallResult();
      if (mounted && notice != null) {
        state = state.copyWith(installNotice: notice);
      }
    } catch (_) {/* Keep the player usable when preference storage fails. */}
  }

  Future<void> _save(Future<bool>? Function() write) async {
    try {
      await write();
    } catch (_) {/* Session settings still apply. */}
  }

  Future<void> setAutoCheck(bool value) async {
    await ready;
    if (!mounted) return;
    state = state.copyWith(autoCheck: value);
    await _save(() => _prefs?.setBool(_autoKey, value));
    if (value) await check();
  }

  Future<void> setAutoDownload(bool value) async {
    await ready;
    if (!mounted) return;
    state = state.copyWith(autoDownload: value);
    await _save(() => _prefs?.setBool(_downloadKey, value));
    if (value) {
      await _autoDownload();
    } else {
      cancelDownload();
    }
  }

  Future<void> ignoreRelease() async {
    final identity = state.release?.identity;
    if (identity == null) return;
    cancelDownload();
    state = state.copyWith(ignoredVersion: identity);
    await _save(() => _prefs?.setString(_ignoredKey, identity));
  }

  Future<void> _autoDownload() async {
    if (_packages != null &&
        state.autoDownload &&
        state.release != null &&
        state.release!.identity != state.ignoredVersion &&
        state.status == UpdateStatus.available) {
      await downloadUpdate();
    }
  }

  Future<void> check({bool manual = false}) async {
    await ready;
    if (!mounted || state.busy) return;
    final now = _now();
    // Manual checks and restarts must not bypass a rejection or a click burst.
    if (_retryNotBefore != null && now.isBefore(_retryNotBefore!)) return;
    if (_lastAttempt != null &&
        now.difference(_lastAttempt!) < const Duration(seconds: 5)) {
      return;
    }
    if (!manual) {
      if (!state.autoCheck) return;
      final last = state.lastChecked;
      if (last != null &&
          !now.isBefore(last) &&
          now.difference(last) < checkInterval) {
        await _autoDownload();
        return;
      }
      if (_lastAttempt != null &&
          !now.isBefore(_lastAttempt!) &&
          now.difference(_lastAttempt!) < const Duration(minutes: 30)) {
        return;
      }
    }
    _lastAttempt = now;
    state = state.copyWith(status: UpdateStatus.checking);
    await _save(() => _prefs?.setString(_attemptKey, now.toIso8601String()));
    try {
      final installed = await _installedVersion();
      final release = await _service.latestRelease();
      if (!mounted) return;
      final newer = release != null && _newer(release, installed);
      _retryNotBefore = null;
      await _save(() => _prefs?.remove(_retryKey));
      if (!mounted) return;
      if (state.release?.identity != release?.identity) _downloaded = null;
      state = AppUpdateState(
          status: release == null
              ? UpdateStatus.noRelease
              : newer
                  ? (_downloaded != null
                      ? UpdateStatus.ready
                      : UpdateStatus.available)
                  : UpdateStatus.current,
          autoCheck: state.autoCheck,
          autoDownload: state.autoDownload,
          release: newer ? release : null,
          lastChecked: now,
          ignoredVersion: state.ignoredVersion,
          installNotice: state.installNotice);
      await _save(() => newer
          ? _prefs?.setString(_releaseKey, jsonEncode(release.toJson()))
          : _prefs?.remove(_releaseKey));
      await _save(() => _prefs?.setString(_checkedKey, now.toIso8601String()));
      await _autoDownload();
    } catch (error) {
      if (mounted) {
        var delay = error is UpdateCheckException ? error.retryAfter : null;
        if (delay == null || delay < const Duration(seconds: 30)) {
          delay = const Duration(seconds: 30);
        }
        _retryNotBefore = _now().add(delay);
        state = state.copyWith(
            status: UpdateStatus.failed,
            error: error is UpdateCheckException
                ? error.message
                : '无法读取客户端版本或检查更新，请稍后重试');
        await _save(() =>
            _prefs?.setString(_retryKey, _retryNotBefore!.toIso8601String()));
      }
    }
  }

  Future<void> downloadUpdate() async {
    await ready;
    if (!mounted) return;
    if (_retryNotBefore != null && _now().isBefore(_retryNotBefore!)) {
      state =
          state.copyWith(status: UpdateStatus.failed, error: '更新请求正在冷却，请稍后重试');
      return;
    }
    final release = state.release;
    final packages = _packages;
    if (release == null || state.busy) return;
    if (packages == null) {
      state = state.copyWith(status: UpdateStatus.failed, error: '更新安装服务不可用');
      return;
    }
    final generation = ++_downloadGeneration;
    state = state.copyWith(status: UpdateStatus.downloading, progress: 0);
    try {
      final file = await packages.download(release, (progress) {
        if (mounted &&
            generation == _downloadGeneration &&
            (progress >= 1 || progress - state.progress >= .01)) {
          state = state.copyWith(progress: progress);
        }
      });
      if (mounted && generation == _downloadGeneration) {
        _downloaded = file;
        _retryNotBefore = null;
        state = state.copyWith(status: UpdateStatus.ready, progress: 1);
        await _save(() => _prefs?.remove(_retryKey));
      }
    } catch (error) {
      if (mounted && generation == _downloadGeneration) {
        var delay = error is UpdateCheckException ? error.retryAfter : null;
        if (delay == null || delay < const Duration(seconds: 30)) {
          delay = const Duration(seconds: 30);
        }
        _retryNotBefore = _now().add(delay);
        state = state.copyWith(
            status: UpdateStatus.failed,
            error:
                error is UpdateCheckException ? error.message : '下载更新失败，请重试');
        await _save(() =>
            _prefs?.setString(_retryKey, _retryNotBefore!.toIso8601String()));
      }
    }
  }

  void cancelDownload() {
    if (state.status != UpdateStatus.downloading) return;
    _downloadGeneration++;
    _packages?.cancel();
    state = state.copyWith(status: UpdateStatus.available, progress: 0);
  }

  Future<void> installUpdate() async {
    if (state.status != UpdateStatus.ready ||
        _downloaded == null ||
        state.release == null ||
        _packages == null) {
      return;
    }
    state = state.copyWith(status: UpdateStatus.installing);
    try {
      await _packages.install(state.release!, _downloaded!);
      if (mounted) {
        state = state.copyWith(
            status: UpdateStatus.ready, installNotice: '已打开系统安装器，请按系统提示完成更新');
      }
    } catch (error) {
      if (mounted) {
        state = state.copyWith(
            status: UpdateStatus.ready,
            error: error is UpdateCheckException
                ? error.message
                : '安装失败，原版本保持可用，请重试');
      }
    }
  }

  @override
  void dispose() {
    _downloadGeneration++;
    _packages?.cancel();
    super.dispose();
  }
}

final appUpdateProvider =
    StateNotifierProvider<AppUpdateController, AppUpdateState>((ref) {
  final service = AppUpdateService();
  final packages = UpdatePackageManager();
  ref.onDispose(service.close);
  ref.onDispose(packages.close);
  return AppUpdateController(
      service: service,
      packages: packages,
      installedVersion: () => ref.read(clientVersionProvider.future));
});
