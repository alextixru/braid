/// Точки подключения экранов тикета 11: «Движки», «Статистика» (только
/// админ), «О программе», значок в меню-баре, диалог капчи. Слоты заполнены в
/// [kAppExtensions]; остальной код тикета 10 они не меняют.
library;

import 'dart:async';

import 'package:braid/controller/braid_controller.dart';
import 'package:braid/screens/about/about_screen.dart';
import 'package:braid/screens/engines/engine_events_host.dart';
import 'package:braid/screens/engines/engine_services.dart';
import 'package:braid/screens/engines/engines_screen.dart';
import 'package:braid/screens/stats/stats_screen.dart';
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

/// Подключённые надстройки: экраны «Движки», «Статистика», «О программе»,
/// диалог капчи поверх приложения и запуск значка в меню-баре, входа VK и
/// проверки уведомлений, когда ядро готово.
const kAppExtensions = AppExtensions(
  engines: buildEnginesScreen,
  stats: buildStatsScreen,
  about: buildAboutScreen,
  wrapApp: wrapWithEngineEvents,
  onReady: startEngineServices,
);
