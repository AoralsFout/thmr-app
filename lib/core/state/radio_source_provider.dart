import 'dart:async';

import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../api/radio_api.dart';
import '../network/radio_sources.dart';
import 'settings_provider.dart';
import '../player/desktop_session.dart';
import '../network/network_probe.dart';

export '../network/radio_sources.dart';

class RadioSourceState {
  const RadioSourceState({
    this.preference = RadioSourcePreference.automatic,
    this.selected = const RadioSource(
        RadioSourcePreference.source1, '源 1', 'https://radio.yunmoan.cn'),
    this.testing = false,
    this.tested = false,
    this.measurements = const [],
    this.testedAt,
    this.notice,
  });

  final RadioSourcePreference preference;
  final RadioSource selected;
  final bool testing;
  final bool tested;
  final List<SourceMeasurement> measurements;
  final DateTime? testedAt;
  final String? notice;

  bool get ready => preference != RadioSourcePreference.automatic || tested;
}

typedef SourceTesterFactory = RadioSourceTester Function();

class RadioSourceController extends StateNotifier<RadioSourceState> {
  RadioSourceController.remote(DesktopSession session)
      : _testerFactory = RadioSourceTester.new,
        _remote = session,
        super(const RadioSourceState()) {
    _applyRemote(session.snapshot);
    _remoteSubscription = session.events.listen(_applyRemote);
  }

  DesktopSession? _remote;
  StreamSubscription<Map<String, dynamic>>? _remoteSubscription;
  void _applyRemote(Map<String, dynamic> snapshot) {
    if (!mounted) return;
    final preference = switch (snapshot['source']) {
      'source1' => RadioSourcePreference.source1,
      'source2' => RadioSourcePreference.source2,
      _ => RadioSourcePreference.automatic,
    };
    final base = snapshot['radioBaseUrl'] as String;
    final selected = base == radioSources[1].baseUrl
        ? RadioSourcePreference.source2
        : RadioSourcePreference.source1;
    final measurements = <SourceMeasurement>[];
    for (final result
        in (snapshot['sourceMeasurements'] as List? ?? const [])) {
      final sample = (result as Map).cast<String, dynamic>();
      final source = radioSources[sample['source'] == 'source2' ? 1 : 0];
      measurements.add(SourceMeasurement(
          source,
          NetworkProbeResult(
            uri: Uri.parse(
                '${source.baseUrl}/radio/${snapshot['stationId']}/${snapshot['profileId']}'),
            statusCode: (sample['statusCode'] as num?)?.toInt(),
            elapsed: Duration(
                milliseconds: (sample['elapsedMs'] as num?)?.toInt() ?? 0),
            bytesRead: (sample['bytesRead'] as num?)?.toInt() ?? 0,
            contentType: sample['contentType'] as String? ?? '',
            error: sample['error'] as String?,
          )));
    }
    state = RadioSourceState(
        preference: preference,
        selected: RadioSource(selected,
            selected == RadioSourcePreference.source2 ? '源 2' : '源 1', base),
        tested: true,
        testing: snapshot['sourceTesting'] == true,
        measurements: measurements,
        notice: snapshot['metadataError'] as String?);
  }

  RadioSourceController({
    RadioSourcePreference preference = RadioSourcePreference.automatic,
    SourceTesterFactory? testerFactory,
  })  : _testerFactory = testerFactory ?? RadioSourceTester.new,
        super(RadioSourceState(
          preference: preference,
          selected:
              radioSources[preference == RadioSourcePreference.source2 ? 1 : 0],
        ));

  final SourceTesterFactory _testerFactory;
  RadioSourceTester? _tester;
  Future<void>? _inFlight;
  int _generation = 0;

  void setPreference(RadioSourcePreference preference) {
    if (_remote != null) {
      unawaited(_remote!.execute('set_source', {
        'source': preference == RadioSourcePreference.automatic
            ? 'auto'
            : preference.name
      }));
      return;
    }
    _generation++;
    _tester?.close();
    _tester = null;
    _inFlight = null;
    state = RadioSourceState(
      preference: preference,
      selected: preference == RadioSourcePreference.automatic
          ? state.selected
          : radioSources[preference == RadioSourcePreference.source2 ? 1 : 0],
      tested: state.tested,
      measurements: state.measurements,
      testedAt: state.testedAt,
    );
    if (preference == RadioSourcePreference.automatic) unawaited(testSources());
  }

  Future<void> ensureSelected() {
    if (_remote != null) return Future.value();
    if (state.ready) return Future.value();
    return _inFlight ?? testSources();
  }

  Future<void> testSources() {
    if (_remote != null) return _remote!.execute('test_sources').then((_) {});
    if (_inFlight != null) return _inFlight!;
    final future = _measure();
    _inFlight = future;
    return future;
  }

  Future<void> _measure() async {
    final generation = ++_generation;
    final tester = _testerFactory();
    _tester = tester;
    state = RadioSourceState(
      preference: state.preference,
      selected: state.selected,
      testing: true,
      tested: state.tested,
      measurements: state.measurements,
      testedAt: state.testedAt,
    );
    try {
      final results = await tester.measure();
      if (!mounted || generation != _generation) return;
      final fastest = fastestRadioSource(results);
      state = RadioSourceState(
        preference: state.preference,
        selected: state.preference == RadioSourcePreference.automatic
            ? chooseRadioSource(results, state.selected)
            : state.selected,
        tested: true,
        measurements: results,
        testedAt: DateTime.now(),
        notice: fastest == null ? '两路测速均未成功，暂用当前来源，可稍后重测或手动选择。' : null,
      );
    } catch (_) {
      if (!mounted || generation != _generation) return;
      state = RadioSourceState(
        preference: state.preference,
        selected: state.selected,
        tested: true,
        measurements: state.measurements,
        testedAt: DateTime.now(),
        notice: '测速未完成，暂用当前来源，可稍后重测。',
      );
    } finally {
      tester.close();
      if (generation == _generation) {
        _tester = null;
        _inFlight = null;
      }
    }
  }

  @override
  void dispose() {
    _remoteSubscription?.cancel();
    _generation++;
    _tester?.close();
    super.dispose();
  }
}

final radioSourceProvider =
    StateNotifierProvider<RadioSourceController, RadioSourceState>((ref) {
  final remote = DesktopSession.current;
  if (remote != null) return RadioSourceController.remote(remote);
  final controller = RadioSourceController(
    preference: ref.read(settingsProvider).radioSource,
  );
  ref.listen(settingsProvider.select((settings) => settings.radioSource),
      (_, next) => controller.setPreference(next));
  return controller;
});

final radioApiProvider = Provider<RadioApi>((ref) => RadioApi(
      baseUrl: ref
          .watch(radioSourceProvider.select((state) => state.selected.baseUrl)),
    ));
