/// Точки подключения экранов тикета 11: «Движки», «Статистика» (только
/// админ), «О программе», значок в меню-баре, диалог капчи. Пока слот пуст,
/// пункта меню нет. Тикет 11 заполняет [kAppExtensions] — ровно здесь, больше
/// ничего в тикете 10 править не нужно.
library;

import 'dart:async';

import 'package:braid/controller/braid_controller.dart';
import 'package:flutter/widgets.dart';

class AppExtensions {
  const AppExtensions({
    this.engines,
    this.stats,
    this.about,
    this.wrapApp,
    this.onReady,
  });

  /// Экран «Движки»: состояния, закрепление, «Проверить связь», CSQTT.
  final WidgetBuilder? engines;

  /// Экран «Статистика»; в меню появляется только при `CoreState.admin`.
  final WidgetBuilder? stats;

  /// Экран «О программе».
  final WidgetBuilder? about;

  /// Обёртка над всем приложением: слушатели событий ядра, которым нужен
  /// диалог поверх любого экрана (капча VK). Получает дерево приложения.
  final Widget Function(BuildContext context, Widget app)? wrapApp;

  /// Вызывается один раз, когда ядро готово: значок в меню-баре, уведомления.
  final FutureOr<void> Function(BraidController controller)? onReady;
}

/// Подключённые на сегодня надстройки. Тикет 11: впиши сюда свои экраны.
const kAppExtensions = AppExtensions();
