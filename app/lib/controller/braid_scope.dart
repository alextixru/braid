import 'package:braid/app_extensions.dart';
import 'package:braid/controller/braid_controller.dart';
import 'package:flutter/widgets.dart';

/// Контроллер и точки подключения экранов тикета 11 для всего дерева виджетов.
/// Виджет, который читает состояние через [of], перестраивается на каждый кадр
/// ядра; тот, кому нужны только команды, берёт [read].
class BraidScope extends InheritedNotifier<BraidController> {
  const BraidScope({
    super.key,
    required BraidController controller,
    required this.extensions,
    required this.navigatorKey,
    required super.child,
  }) : super(notifier: controller);

  final AppExtensions extensions;

  /// Ключ навигатора приложения: диалоги поверх любого экрана (капча VK).
  final GlobalKey<NavigatorState> navigatorKey;

  static BraidScope _scope(BuildContext context, {required bool listen}) {
    final scope = listen
        ? context.dependOnInheritedWidgetOfExactType<BraidScope>()
        : context.getInheritedWidgetOfExactType<BraidScope>();
    assert(scope != null, 'BraidScope не найден: оберни дерево в BraidApp');
    return scope!;
  }

  /// Контроллер; виджет перестраивается при его изменении.
  static BraidController of(BuildContext context) =>
      _scope(context, listen: true).notifier!;

  /// Контроллер без подписки на изменения — для обработчиков нажатий.
  static BraidController read(BuildContext context) =>
      _scope(context, listen: false).notifier!;

  /// Экраны и надстройки тикета 11.
  static AppExtensions extensionsOf(BuildContext context) =>
      _scope(context, listen: false).extensions;

  /// Навигатор приложения; контекст `wrapApp` стоит над ним, и без ключа
  /// показать диалог было бы нечем.
  static GlobalKey<NavigatorState> navigatorKeyOf(BuildContext context) =>
      _scope(context, listen: false).navigatorKey;
}
