import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:thmr_app/core/api/radio_api.dart';
import 'package:thmr_app/core/models/models.dart';
import 'package:thmr_app/core/state/station_provider.dart';

class _DelayedRadioApi extends RadioApi {
  final requests = <Completer<StationStatus>>[];

  @override
  Future<StationStatus> status(String stationId) {
    final request = Completer<StationStatus>();
    requests.add(request);
    return request.future;
  }
}

StationStatus _status(int listeners) => StationStatus(
      station: const StationInfo(
        id: 'main',
        name: 'THMR',
        enabled: true,
        public: null,
        streamAuthEnabled: false,
        bitrates: [],
      ),
      listeners: listeners,
      uniqueIps: listeners,
      current: null,
      previous: null,
      queue: const [],
    );

void main() {
  test('manual refresh joins the pending poll instead of opening another',
      () async {
    final api = _DelayedRadioApi();
    final poller = StationPoller(api, 'main');
    addTearDown(poller.dispose);

    final refresh = poller.refresh();
    expect(api.requests, hasLength(1));
    api.requests[0].complete(_status(2));
    await refresh;
    expect(poller.state.status?.listeners, 2);

    await poller.refresh();
    expect(api.requests, hasLength(1));
  });

  test('failed polls back off even when refresh is clicked repeatedly',
      () async {
    final api = _DelayedRadioApi();
    final poller = StationPoller(api, 'main');
    addTearDown(poller.dispose);

    final refreshes = List.generate(100, (_) => poller.refresh());
    expect(api.requests, hasLength(1));
    api.requests[0].completeError(ApiException(429, 'limited'));
    await Future.wait(refreshes);
    await Future<void>.delayed(Duration.zero);
    expect(poller.state.error, isA<ApiException>());
    for (var i = 0; i < 100; i++) {
      await poller.refresh();
    }
    expect(api.requests, hasLength(1));
  });
}
