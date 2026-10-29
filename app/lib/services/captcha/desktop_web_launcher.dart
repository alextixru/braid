/// Окно браузера на десктопе: `desktop_webview_window` (WKWebView на macOS).
library;

import 'package:braid/services/captcha/web_session.dart';
import 'package:desktop_webview_window/desktop_webview_window.dart';
import 'package:flutter/widgets.dart';

class DesktopWebLauncher implements WebLauncher {
  const DesktopWebLauncher();

  @override
  Future<bool> isAvailable() => WebviewWindow.isWebviewAvailable();

  @override
  Future<WebSession> open(WebRequest request) async {
    final webview = await WebviewWindow.create(
      configuration: CreateConfiguration(
        title: request.title,
        windowWidth: request.width,
        windowHeight: request.height,
      ),
    );
    await webview.launch(request.url.toString());
    return _DesktopSession(webview);
  }
}

class _DesktopSession implements WebSession {
  _DesktopSession(this._webview);

  final Webview _webview;

  /// Окно ОС: страницы в дереве виджетов нет.
  @override
  Widget? get embedded => null;

  @override
  Future<String?> currentUrl() async {
    // Адрес с фрагментом (`…#access_token=`) надёжнее читать у самой страницы.
    final fromPage = await evaluate('window.location.href');
    if (fromPage != null && fromPage.isNotEmpty) return fromPage;
    return _webview.navigationState.value.url;
  }

  @override
  Future<String?> evaluate(String script) async {
    final result = await _webview.evaluateJavaScript(script);
    return result?.toString();
  }

  @override
  Future<void> get closed => _webview.onClose;

  @override
  Future<void> close() => _webview.close();
}
