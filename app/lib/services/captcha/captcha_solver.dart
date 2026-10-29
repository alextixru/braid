/// Капча VK (ист. 16): страница капчи в браузере и `success_token` из ответов.
///
/// Ядро просит капчу событием `NeedsCaptcha{url, session}`. Пользователь
/// решает её на странице VK; после решения страница сама отправляет запрос,
/// ответ которого несёт `success_token`. Токена нет в адресе страницы, поэтому
/// в страницу ставится перехватчик `fetch` и `XMLHttpRequest`, а токен ищется в
/// собранных ответах — так делает и FOCSQ. Результат уходит ядру строкой:
/// токен или `error:*` (ядро считает такую строку неудачей).
library;

import 'package:braid/services/captcha/web_flow.dart';
import 'package:braid/services/captcha/web_session.dart';

class CaptchaSolver {
  CaptchaSolver({
    required this.launcher,
    this.interval = const Duration(milliseconds: 400),
    this.timeout = const Duration(seconds: 120),
  });

  /// Окно закрыто или пользователь отказался.
  static const closed = 'error:closed';

  /// Не решили за отведённое время.
  static const timedOut = 'error:timeout';

  /// Браузер недоступен или страница не открылась.
  static const webviewFailed = 'error:webview';

  /// Ставит перехватчик ответов страницы. Повторный вызов ничего не меняет:
  /// страница могла перезагрузиться, поэтому ставим на каждый опрос. Собранное
  /// ограничено 64 КБ с конца — токен приходит в последних ответах.
  static const hookScript = '''
(function () {
  if (window.__braidHooked) { return 'hooked'; }
  window.__braidHooked = true;
  window.__braidCapture = '';
  var keep = function (text) {
    try { window.__braidCapture = (window.__braidCapture + text).slice(-65536); } catch (e) {}
  };
  var realFetch = window.fetch;
  if (realFetch) {
    window.fetch = function () {
      return realFetch.apply(this, arguments).then(function (response) {
        try { response.clone().text().then(keep); } catch (e) {}
        return response;
      });
    };
  }
  var realOpen = XMLHttpRequest.prototype.open;
  XMLHttpRequest.prototype.open = function () {
    this.addEventListener('load', function () {
      try { keep(this.responseText); } catch (e) {}
    });
    return realOpen.apply(this, arguments);
  };
  return 'hooked';
})()
''';

  static const readScript = 'window.__braidCapture || ""';

  // `"success_token":"…"` и то же самое внутри строки с экранированием.
  static final RegExp _token = RegExp(
    r'success_token\\?"\s*:\s*\\?"([0-9a-zA-Z_\-]+)',
  );

  final WebLauncher launcher;

  /// Как часто спрашивать страницу.
  final Duration interval;

  /// Сколько ждать решения.
  final Duration timeout;

  /// Открыть страницу капчи и ждать токена. Не бросает: любой сбой — `error:*`.
  Future<WebFlow<String>> begin(Uri url) async {
    if (url.scheme != 'https' || url.host.isEmpty) {
      return WebFlow.failed(webviewFailed);
    }
    try {
      if (!await launcher.isAvailable()) return WebFlow.failed(webviewFailed);
      final session = await launcher.open(
        WebRequest(
          url: url,
          title: 'Подтвердите, что вы не робот',
          width: 420,
          height: 620,
        ),
      );
      return WebFlow.poll(
        session: session,
        probe: _probe,
        onClosed: closed,
        onTimeout: timedOut,
        interval: interval,
        timeout: timeout,
      );
    } on Object {
      return WebFlow.failed(webviewFailed);
    }
  }

  Future<String?> _probe(WebSession session) async {
    await session.evaluate(hookScript);
    final captured = await session.evaluate(readScript);
    if (captured == null || captured.isEmpty) return null;
    return _token.firstMatch(captured)?.group(1);
  }
}
