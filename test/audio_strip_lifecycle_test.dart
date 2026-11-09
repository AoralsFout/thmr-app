import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:thmr_app/core/player/spectrum.dart';
import 'package:thmr_app/features/listen/widgets/now_playing_panel.dart';

class _SpectrumSource implements SpectrumSource {
  int starts = 0;
  int stops = 0;
  int polls = 0;

  @override
  void start() => starts++;

  @override
  void stop() => stops++;

  @override
  Float32List poll(int bands) {
    polls++;
    return Float32List.fromList(List.filled(bands, polls.isOdd ? 0.9 : 0.2));
  }
}

Widget _strip(_SpectrumSource source,
    {bool enabled = true, bool playing = true}) {
  return MaterialApp(
    home: TickerMode(
      enabled: enabled,
      child: AudioStrip(playing: playing, source: source),
    ),
  );
}

Future<void> _frames(WidgetTester tester) async {
  for (var i = 0; i < 3; i++) {
    await tester.pump(const Duration(milliseconds: 60));
  }
}

CustomPainter? _painter(WidgetTester tester) => tester
    .widget<CustomPaint>(find.descendant(
      of: find.byType(AudioStrip),
      matching: find.byType(CustomPaint),
    ))
    .painter;

void main() {
  for (final platform in [
    TargetPlatform.windows,
    TargetPlatform.linux,
    TargetPlatform.macOS,
  ]) {
    testWidgets('$platform keeps polling and painting after losing focus',
        (tester) async {
      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
      final source = _SpectrumSource();
      await tester.pumpWidget(_strip(source));
      await _frames(tester);
      expect(source.polls, greaterThan(0));
      final polls = source.polls;
      final painter = _painter(tester);

      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.inactive);
      await _frames(tester);
      expect(source.stops, 0);
      expect(source.polls, greaterThan(polls));
      expect(_painter(tester), isNot(same(painter)));

      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.hidden);
      final hiddenPolls = source.polls;
      await _frames(tester);
      expect(source.stops, 1);
      expect(source.polls, hiddenPolls);

      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.inactive);
      await _frames(tester);
      expect(source.starts, 2);
      expect(source.polls, greaterThan(hiddenPolls));

      await tester.pumpWidget(const SizedBox.shrink());
      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
    }, variant: TargetPlatformVariant.only(platform));
  }

  for (final platform in [TargetPlatform.android, TargetPlatform.iOS]) {
    testWidgets('$platform stops on inactive and restarts on resume',
        (tester) async {
      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
      final source = _SpectrumSource();
      await tester.pumpWidget(_strip(source));
      await _frames(tester);
      final polls = source.polls;
      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.inactive);
      await _frames(tester);
      expect(source.stops, 1);
      expect(source.polls, polls);

      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
      await _frames(tester);
      expect(source.starts, 2);
      expect(source.polls, greaterThan(polls));
      await tester.pumpWidget(const SizedBox.shrink());
    }, variant: TargetPlatformVariant.only(platform));
  }

  for (final state in [
    AppLifecycleState.inactive,
    AppLifecycleState.hidden,
    AppLifecycleState.paused,
    AppLifecycleState.detached,
  ]) {
    testWidgets('desktop strip mounted in $state respects visibility',
        (tester) async {
      tester.binding.handleAppLifecycleStateChanged(state);
      final source = _SpectrumSource();
      await tester.pumpWidget(_strip(source));
      await _frames(tester);
      if (state == AppLifecycleState.inactive) {
        expect(source.starts, 1);
        expect(source.polls, greaterThan(0));
      } else {
        expect(source.starts, 0);
        expect(source.polls, 0);
      }
      await tester.pumpWidget(const SizedBox.shrink());
      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
    }, variant: TargetPlatformVariant.only(TargetPlatform.windows));
  }

  testWidgets('unfocused desktop respects disabled tickers and playback pause',
      (tester) async {
    tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
    final source = _SpectrumSource();
    await tester.pumpWidget(_strip(source));
    await _frames(tester);
    await tester.pumpWidget(_strip(source, enabled: false));
    tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.inactive);
    final polls = source.polls;
    await _frames(tester);
    expect(source.stops, 1);
    expect(source.polls, polls);
    expect(source.starts, 1);

    await tester.pumpWidget(_strip(source));
    await _frames(tester);
    expect(source.starts, 2);
    expect(source.polls, greaterThan(polls));

    await tester.pumpWidget(_strip(source, playing: false));
    final pausedPolls = source.polls;
    tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
    tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.inactive);
    await _frames(tester);
    expect(source.stops, 2);
    expect(source.polls, pausedPolls);
    expect(source.starts, 2);
    await tester.pumpWidget(const SizedBox.shrink());
    tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
  }, variant: TargetPlatformVariant.only(TargetPlatform.windows));
}
