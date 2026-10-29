/// Разрешение на уведомления (оговорка тикета 09): на Android 13+ без
/// `POST_NOTIFICATIONS` не показываются ни постоянное уведомление сервиса, ни
/// значок в шторке, ни сообщения о переключении движка.
///
/// Сами уведомления шлют сервис (Kotlin) и ядро через `Platform::notify`
/// (только при включённой настройке — ист. 59); окно приложения лишь просит
/// разрешение и подсказывает, как его вернуть. Запрос — один раз, до первого
/// подключения: у `connect()` нет места для вставки, поэтому спрашиваем, как
/// только в приложении появилась подписка, то есть раньше, чем пользователь
/// сможет нажать «Подключить» и запустить сервис.
library;

import 'dart:async';

import 'package:braid/app.dart' show appName;
import 'package:braid/controller/braid_controller.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter_local_notifications/flutter_local_notifications.dart';
import 'package:shared_preferences/shared_preferences.dart';

/// Текст подсказки: что пропадёт без разрешения и куда идти.
const String kNotificationHint =
    'Уведомления $appName выключены в Android — без них нет значка в шторке и '
    'сообщений о переключении. Включи их в настройках приложения.';

abstract interface class NotificationPermission {
  /// Уведомления приложения разрешены (до Android 13 — включены).
  Future<bool> isGranted();

  /// Системный запрос `POST_NOTIFICATIONS`; `true` — разрешение выдано.
  Future<bool> request();

  /// Системные настройки уведомлений этого приложения.
  Future<bool> openSettings();
}

/// `flutter_local_notifications`: `requestNotificationsPermission()`,
/// `areNotificationsEnabled()`, `openAppNotificationSettings()`.
class PluginNotificationPermission implements NotificationPermission {
  PluginNotificationPermission([FlutterLocalNotificationsPlugin? plugin])
    : _plugin = plugin ?? FlutterLocalNotificationsPlugin();

  final FlutterLocalNotificationsPlugin _plugin;

  AndroidFlutterLocalNotificationsPlugin? get _android => _plugin
      .resolvePlatformSpecificImplementation<
        AndroidFlutterLocalNotificationsPlugin
      >();

  @override
  Future<bool> isGranted() async =>
      await _android?.areNotificationsEnabled() ?? true;

  @override
  Future<bool> request() async {
    await _android?.requestNotificationsPermission();
    return isGranted();
  }

  @override
  Future<bool> openSettings() async =>
      await _android?.openAppNotificationSettings() ?? false;
}

/// Спрашивали ли уже: после отказа второй раз не мучаем (а после второго
/// отказа Android всё равно не показывает диалог).
abstract interface class NotificationPrefs {
  Future<bool> asked();

  Future<void> setAsked();
}

class SharedNotificationPrefs implements NotificationPrefs {
  const SharedNotificationPrefs();

  static const _key = 'notification_permission_asked';

  @override
  Future<bool> asked() async =>
      (await SharedPreferences.getInstance()).getBool(_key) ?? false;

  @override
  Future<void> setAsked() async {
    await (await SharedPreferences.getInstance()).setBool(_key, true);
  }
}

class NotificationPermissionGuard extends ChangeNotifier {
  NotificationPermissionGuard({
    required this.controller,
    required this.permission,
    required this.prefs,
  });

  final BraidController controller;
  final NotificationPermission permission;
  final NotificationPrefs prefs;

  bool? _granted;
  bool _askedThisRun = false;
  bool _wantedSwitchNotices = false;
  bool _started = false;
  bool _disposed = false;

  /// Разрешено ли; `null` — ещё не проверяли.
  bool? get granted => _granted;

  /// Разрешения нет — на экране нужна подсказка.
  bool get missing => _granted == false;

  /// Проверить разрешение и, если нужно и пора, спросить.
  Future<void> start() async {
    if (_started) return;
    _started = true;
    _wantedSwitchNotices = controller.settings.notifySwitches;
    controller.addListener(_onController);
    await recheck();
    await _askIfDue();
  }

  /// Заново прочитать разрешение: пользователь мог выдать его в настройках.
  Future<void> recheck() async {
    final granted = await _safely(permission.isGranted) ?? false;
    if (granted == _granted) return;
    _granted = granted;
    if (!_disposed) notifyListeners();
  }

  Future<void> openSettings() async {
    await _safely(permission.openSettings);
  }

  void _onController() {
    final wants = controller.settings.notifySwitches;
    if (wants && !_wantedSwitchNotices && missing) {
      controller.showNotice(kNotificationHint);
    }
    _wantedSwitchNotices = wants;
    unawaited(_askIfDue());
  }

  Future<void> _askIfDue() async {
    if (_disposed || _askedThisRun || _granted != false) return;
    // Пока идёт онбординг, сервис запускать нечем и просить рано.
    if (!controller.state.hasSubscription) return;
    _askedThisRun = true;
    if (await prefs.asked()) return;
    await prefs.setAsked();
    await _safely(permission.request);
    await recheck();
  }

  /// Платформенный слой может не отвечать (нет Activity, нет плагина) —
  /// приложению это не мешает, а подсказка останется.
  Future<T?> _safely<T>(Future<T> Function() call) async {
    try {
      return await call();
    } on Object {
      return null;
    }
  }

  @override
  void dispose() {
    _disposed = true;
    if (_started) controller.removeListener(_onController);
    super.dispose();
  }
}
