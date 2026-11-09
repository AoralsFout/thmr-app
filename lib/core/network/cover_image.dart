import 'dart:ui' as ui;

import 'package:dio/dio.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/painting.dart';

import 'request_policy.dart';

/// Uses Flutter's decoded image cache, plus the same pacing and circuit breaker
/// as API requests. A queue of thumbnails must not bypass host protection.
@immutable
class CoverImage extends ImageProvider<CoverImage> {
  const CoverImage(this.url);
  final String url;
  static final _http = ApiHttpClient(
      client: Dio(BaseOptions(
          connectTimeout: const Duration(seconds: 8),
          receiveTimeout: const Duration(seconds: 12))));

  @override
  Future<CoverImage> obtainKey(ImageConfiguration configuration) =>
      SynchronousFuture(this);

  @override
  ImageStreamCompleter loadImage(CoverImage key, ImageDecoderCallback decode) =>
      MultiFrameImageStreamCompleter(codec: _load(decode), scale: 1);

  Future<ui.Codec> _load(ImageDecoderCallback decode) async {
    final response = await _http.get<List<int>>(url,
        options: Options(responseType: ResponseType.bytes));
    final bytes = Uint8List.fromList(response.data!);
    return decode(await ui.ImmutableBuffer.fromUint8List(bytes));
  }

  @override
  bool operator ==(Object other) => other is CoverImage && other.url == url;
  @override
  int get hashCode => url.hashCode;
}
