// Разрешение на уведомления через настоящий плагин `flutter_local_notifications`:
// канал подменён, проверяется, что обёртка зовёт именно то, что просит тикет —
// `requestNotificationsPermission()` и `areNotificationsEnabled()`.

import 'package:braid/tray/notification_permission.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter_local_notifications/flutter_local_notifications.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';

const channel = MethodChannel('dexterous.com/flutter/local_notifications');

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  late List<String> calls;
  late bool enabled;

  setUp(() {
    calls = [];
    enabled = false;
    FlutterLocalNotificationsPlatform.instance =
        AndroidFlutterLocalNotificationsPlugin();
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(channel, (call) async {
          calls.add(call.method);
          return switch (call.method) {
            'areNotificationsEnabled' => enabled,
            // Пользователь отвечает на системный диалог «Разрешить».
            'requestNotificationsPermission' => enabled = true,
            'openAppNotificationSettings' => true,
            _ => null,
          };
        });
  });

  tearDown(() {
    debugDefaultTargetPlatformOverride = null;
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(channel, null);
  });

  test('Android: проверка разрешения читает areNotificationsEnabled', () async {
    debugDefaultTargetPlatformOverride = TargetPlatform.android;
    final permission = PluginNotificationPermission();
    expect(await permission.isGranted(), isFalse);
    expect(calls, ['areNotificationsEnabled']);
  });

  test('Android: запрос идёт через requestNotificationsPermission, ответ — по факту', () async {
    debugDefaultTargetPlatformOverride = TargetPlatform.android;
    final permission = PluginNotificationPermission();
    expect(await permission.request(), isTrue);
    expect(calls, [
      'requestNotificationsPermission',
      'areNotificationsEnabled',
    ]);
  });

  test(
    'Android: «Открыть настройки» открывает настройки уведомлений приложения',
    () async {
      debugDefaultTargetPlatformOverride = TargetPlatform.android;
      expect(await PluginNotificationPermission().openSettings(), isTrue);
      expect(calls, ['openAppNotificationSettings']);
    },
  );

  test('не Android: разрешение «есть», плагин не трогаем', () async {
    debugDefaultTargetPlatformOverride = TargetPlatform.macOS;
    final permission = PluginNotificationPermission();
    expect(await permission.isGranted(), isTrue);
    expect(await permission.request(), isTrue);
    expect(await permission.openSettings(), isFalse);
    expect(calls, isEmpty);
  });

  group('«уже спрашивали»', () {
    test('по умолчанию нет; после отметки — да и после перезапуска', () async {
      SharedPreferences.setMockInitialValues({});
      const prefs = SharedNotificationPrefs();
      expect(await prefs.asked(), isFalse);
      await prefs.setAsked();
      expect(await prefs.asked(), isTrue);

      // «Перезапуск»: новый разбор тех же сохранённых значений.
      expect(await const SharedNotificationPrefs().asked(), isTrue);
    });
  });
}
