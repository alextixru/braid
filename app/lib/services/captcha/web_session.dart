/// Браузер, в котором пользователь проходит вход VK или капчу.
///
/// На Android это встроенный `WebView` (`webview_flutter`) в диалоге, на
/// десктопе — отдельное окно ОС (`desktop_webview_window`). Логика входа и
/// капчи знает только этот интерфейс, поэтому проверяется без настоящих
/// WebView: тест подменяет страницу.
library;

import 'package:flutter/widgets.dart';

/// Что открыть.
class WebRequest {
  const WebRequest({
    required this.url,
    required this.title,
    this.width = 500,
    this.height = 700,
  });

  final Uri url;

  /// Заголовок окна.
  final String title;
  final int width;
  final int height;
}

/// Открытая страница.
abstract interface class WebSession {
  /// Страница внутри приложения (Android): её показывает диалог. У отдельного
  /// окна ОС страницы в дереве виджетов нет — `null`.
  Widget? get embedded;

  /// Адрес страницы верхнего уровня; `null` — пока не загрузилась.
  Future<String?> currentUrl();

  /// Выполнить JavaScript; результат — строка без внешних кавычек.
  Future<String?> evaluate(String script);

  /// Завершается, когда пользователь закрыл окно или страницу.
  Future<void> get closed;

  Future<void> close();
}

abstract interface class WebLauncher {
  /// Можно ли открыть браузер на этой машине (Windows: нужен WebView2).
  Future<bool> isAvailable();

  Future<WebSession> open(WebRequest request);
}
