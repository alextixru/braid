/// Всё, что экраны и надстройки тикета 11 берут снаружи: хранилище VK,
/// браузер для входа и капчи, меню-бар, окно, разрешение на уведомления,
/// файлы выгрузки.
///
/// Один набор на контроллер: экран «Движки», диалог капчи и значок в
/// меню-баре должны видеть одни и те же хеши и один и тот же вход. Настоящие
/// части создаются сами; тесты ставят свои через [install].
library;

import 'dart:async';

import 'package:braid/controller/braid_controller.dart';
import 'package:braid/screens/stats/stats_service.dart';
import 'package:braid/services/captcha/captcha_coordinator.dart';
import 'package:braid/services/captcha/captcha_solver.dart';
import 'package:braid/services/captcha/desktop_web_launcher.dart';
import 'package:braid/services/captcha/in_app_web_launcher.dart';
import 'package:braid/services/captcha/web_session.dart';
import 'package:braid/services/vk/csqtt_credentials.dart';
import 'package:braid/services/vk/vk_credentials.dart';
import 'package:braid/services/vk/vk_oauth.dart';
import 'package:braid/tray/notification_permission.dart';
import 'package:braid/tray/system_tray.dart';
import 'package:braid/tray/tray_icon_renderer.dart';
import 'package:braid/tray/tray_service.dart';
import 'package:braid/tray/window_control.dart';

/// Подключается как `AppExtensions.onReady`: ядро готово — поднять всё, что
/// живёт вместе с приложением (значок в меню-баре, вход VK, уведомления).
Future<void> startEngineServices(BraidController controller) =>
    EngineServices.of(controller).start();

class EngineServices {
  EngineServices({
    required this.controller,
    VkCredentialStore? store,
    WebLauncher? web,
    VkOAuth? oauth,
    CaptchaSolver? solver,
    StatsFiles? statsFiles,
    SystemTray? systemTray,
    TrayIconRenderer? trayIcons,
    WindowControl? window,
    NotificationPermission? notificationPermission,
    NotificationPrefs? notificationPrefs,
  }) : web =
           web ??
           (controller.platform.isAndroid
               ? const InAppWebLauncher()
               : const DesktopWebLauncher()) {
    csqtt = CsqttCredentials(
      controller: controller,
      store: store ?? const SecureVkCredentialStore(),
      oauth: oauth ?? VkOAuth(launcher: this.web),
    );
    captcha = CaptchaCoordinator(
      controller: controller,
      solver: solver ?? CaptchaSolver(launcher: this.web),
    );
    stats = StatsService(
      controller: controller,
      files: statsFiles ?? const SystemStatsFiles(),
    );
    if (controller.platform.isDesktop) {
      tray = TrayService(
        controller: controller,
        tray: systemTray ?? TrayManagerTray(),
        icons: trayIcons ?? CanvasTrayIconRenderer(),
        window: window ?? WindowManagerControl(),
      );
    }
    if (controller.platform.isAndroid) {
      notifications = NotificationPermissionGuard(
        controller: controller,
        permission: notificationPermission ?? PluginNotificationPermission(),
        prefs: notificationPrefs ?? const SharedNotificationPrefs(),
      );
    }
  }

  final BraidController controller;

  /// Браузер для входа VK и капчи.
  final WebLauncher web;

  /// Хеши звонков и вход через VK.
  late final CsqttCredentials csqtt;

  /// Запросы капчи от движка.
  late final CaptchaCoordinator captcha;

  /// Данные экрана «Статистика».
  late final StatsService stats;

  /// Значок в меню-баре; есть только на десктопе.
  TrayService? tray;

  /// Разрешение на уведомления; есть только на Android.
  NotificationPermissionGuard? notifications;

  bool _started = false;

  static final _registry = Expando<EngineServices>('EngineServices');

  /// Набор этого контроллера; настоящий создаётся при первом обращении.
  static EngineServices of(BraidController controller) =>
      _registry[controller] ??= EngineServices(controller: controller);

  /// Поставить свой набор (тесты).
  static void install(EngineServices services) =>
      _registry[services.controller] = services;

  /// Ядро готово: прочитать сохранённое VK, поднять значок в меню-баре и
  /// проверить разрешение на уведомления. Сбой одной части не мешает другим.
  Future<void> start() async {
    if (_started) return;
    _started = true;
    await _guard('вход через VK', csqtt.load);
    final tray = this.tray;
    if (tray != null && controller.caps.menuBar) {
      await _guard('значок в меню-баре', tray.start);
    }
    final notifications = this.notifications;
    if (notifications != null) {
      await _guard('проверку уведомлений', notifications.start);
    }
  }

  Future<void> _guard(String what, Future<void> Function() action) async {
    try {
      await action();
    } on Object catch (error) {
      controller.showNotice(
        'Не удалось запустить $what: ${BraidController.describeError(error)}',
      );
    }
  }

  Future<void> dispose() async {
    csqtt.dispose();
    notifications?.dispose();
    await tray?.dispose();
  }
}
