import 'package:flutter/widgets.dart';

/// Сканер QR (камера): только Android. Интерфейс нужен тестам — камеры у них
/// нет.
abstract interface class QrScanner {
  /// Открыть сканер поверх текущего экрана; вернуть текст кода или `null`,
  /// если пользователь закрыл сканер.
  Future<String?> scan(BuildContext context);
}

/// Камеры нет (десктоп): кнопка «Сканировать QR» не показывается.
class NoQrScanner implements QrScanner {
  const NoQrScanner();

  @override
  Future<String?> scan(BuildContext context) async => null;
}
