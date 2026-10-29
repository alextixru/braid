/// Встроенный браузер на Android: `webview_flutter`. Страница показывается в
/// диалоге, который открывает приложение (`WebSession.embedded`).
library;

import 'dart:async';
import 'dart:convert';

import 'package:braid/services/captcha/web_session.dart';
import 'package:flutter/widgets.dart';
import 'package:webview_flutter/webview_flutter.dart';

class InAppWebLauncher implements WebLauncher {
  const InAppWebLauncher();

  @override
  Future<bool> isAvailable() async => true;

  @override
  Future<WebSession> open(WebRequest request) async {
    final controller = WebViewController()
      ..setJavaScriptMode(JavaScriptMode.unrestricted);
    await controller.loadRequest(request.url);
    return InAppWebSession(controller);
  }
}

class InAppWebSession implements WebSession {
  InAppWebSession(this._controller);

  final WebViewController _controller;
  final _closed = Completer<void>();

  @override
  Widget? get embedded => WebViewWidget(controller: _controller);

  @override
  Future<String?> currentUrl() => _controller.currentUrl();

  @override
  Future<String?> evaluate(String script) async {
    final result = await _controller.runJavaScriptReturningResult(script);
    return plainJavaScriptResult(result);
  }

  @override
  Future<void> get closed => _closed.future;

  @override
  Future<void> close() async {
    if (!_closed.isCompleted) _closed.complete();
  }
}

/// Android отдаёт результат скрипта JSON-строкой («"текст"», «null»), iOS и
/// macOS — значением как есть. Приводим к обычной строке.
String plainJavaScriptResult(Object result) {
  if (result is! String) return result.toString();
  final text = result.trim();
  if (text == 'null') return '';
  if (text.length >= 2 && text.startsWith('"') && text.endsWith('"')) {
    try {
      final decoded = jsonDecode(text);
      if (decoded is String) return decoded;
    } on FormatException {
      return text;
    }
  }
  return text;
}
