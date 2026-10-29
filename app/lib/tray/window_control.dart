/// Окно приложения: скрыть вместо закрытия, показать из меню-бара, выйти.
library;

import 'package:window_manager/window_manager.dart';

/// Окно. Тесты подменяют его записывающей подделкой.
abstract interface class WindowControl {
  /// Закрытие окна больше не завершает приложение: вместо этого вызывается
  /// [onCloseRequested] (приложение скрывает окно и остаётся в меню-баре).
  Future<void> prepare({required void Function() onCloseRequested});

  Future<void> show();

  Future<void> hide();

  /// Завершить приложение.
  Future<void> quit();
}

/// `window_manager`: на macOS без `ensureInitialized` делегат окна не
/// ставится, и `setPreventClose` не действует.
class WindowManagerControl with WindowListener implements WindowControl {
  void Function()? _onCloseRequested;

  @override
  Future<void> prepare({required void Function() onCloseRequested}) async {
    _onCloseRequested = onCloseRequested;
    await windowManager.ensureInitialized();
    await windowManager.setPreventClose(true);
    windowManager.addListener(this);
  }

  @override
  void onWindowClose() => _onCloseRequested?.call();

  @override
  Future<void> show() async {
    await windowManager.show();
    await windowManager.focus();
  }

  @override
  Future<void> hide() => windowManager.hide();

  @override
  Future<void> quit() async {
    windowManager.removeListener(this);
    // `destroy` на macOS — `NSApp.terminate`: закрывает приложение независимо
    // от того, считает ли делегат последнее окно причиной выхода.
    await windowManager.destroy();
  }
}
