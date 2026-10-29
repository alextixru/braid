import 'package:flutter/foundation.dart';

/// Платформа, на которой запущено приложение. Подменяется в тестах: поведение
/// macOS и Android различается (пароль администратора, согласие VPN, сервис).
class PlatformInfo {
  const PlatformInfo(this.os);

  /// Платформа текущего процесса.
  factory PlatformInfo.current() => PlatformInfo(defaultTargetPlatform);

  final TargetPlatform os;

  bool get isAndroid => os == TargetPlatform.android;
  bool get isMacOS => os == TargetPlatform.macOS;
  bool get isDesktop =>
      os == TargetPlatform.macOS ||
      os == TargetPlatform.linux ||
      os == TargetPlatform.windows;
}
