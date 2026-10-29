import 'dart:async';

import 'package:flutter/services.dart';

/// Ссылки `braid://…`, которыми открыли приложение.
///
/// Android разбирает ссылку сам (`MainActivity`: `ACTION_VIEW` → сервис →
/// ядро), здесь для него [NoDeepLinks]. На macOS ссылку приносит родной
/// слой: схема `braid` прописана в `Info.plist`, а `AppDelegate` отдаёт
/// URL каналу `org.braid/links` — `initialLink` (ссылка, которой запустили
/// приложение, до готовности Dart её надо хранить) и вызов `onLink(String)`.
abstract interface class DeepLinks {
  /// Ссылка, которой приложение запущено; `null` — запуск обычный.
  Future<String?> initialLink();

  /// Ссылки, пришедшие, пока приложение работает.
  Stream<String> get links;
}

class NoDeepLinks implements DeepLinks {
  const NoDeepLinks();

  @override
  Future<String?> initialLink() async => null;

  @override
  Stream<String> get links => const Stream.empty();
}

/// Канал `org.braid/links` родного слоя десктопа.
class ChannelDeepLinks implements DeepLinks {
  ChannelDeepLinks([MethodChannel? channel])
    : _channel = channel ?? const MethodChannel(name) {
    _channel.setMethodCallHandler((call) async {
      if (call.method == 'onLink' && call.arguments is String) {
        _controller.add(call.arguments as String);
      }
    });
  }

  static const name = 'org.braid/links';
  final MethodChannel _channel;
  final _controller = StreamController<String>.broadcast();

  @override
  Future<String?> initialLink() async {
    try {
      return await _channel.invokeMethod<String>('initialLink');
    } on MissingPluginException {
      // Родной слой ссылок не подключён — приложение работает без deeplink.
      return null;
    }
  }

  @override
  Stream<String> get links => _controller.stream;
}
