import 'dart:io';

import 'package:path_provider/path_provider.dart';

/// Каталоги приложения.
abstract interface class AppDirs {
  /// Каталог хранилища ядра (подписка, правила, настройки, журнал).
  Future<String> coreDir();
}

class SystemAppDirs implements AppDirs {
  const SystemAppDirs();

  @override
  Future<String> coreDir() async {
    final support = await getApplicationSupportDirectory();
    final dir = Directory('${support.path}${Platform.pathSeparator}core');
    await dir.create(recursive: true);
    return dir.path;
  }
}
