// Разрешение на уведомления (оговорка тикета 09): на Android 13+ без
// POST_NOTIFICATIONS постоянное уведомление сервиса и значок в шторке не
// показываются. Спрашиваем один раз, до первого подключения (до старта
// сервиса); без разрешения — подсказка и кнопка в системные настройки.

import 'package:braid/models/settings.dart';
import 'package:braid/tray/notification_permission.dart';
import 'package:flutter_test/flutter_test.dart';

import 'engines_support.dart';
import 'support/fake_bridge.dart';
import 'support/fakes.dart';
import 'support/pump_app.dart';
import 'support/rig.dart';
import 'support/states.dart';

const hint =
    'Уведомления Braid выключены в Android — без них нет значка в '
    'шторке и сообщений о переключении. Включи их в настройках приложения.';

class Bench {
  Bench(this.rig, this.permission, this.prefs, this.guard);

  final Rig rig;
  final FakeNotificationPermission permission;
  final MemoryNotificationPrefs prefs;
  final NotificationPermissionGuard guard;
}

Future<Bench> androidWith(
  WidgetTester tester, {
  FakeNotificationPermission? permission,
  MemoryNotificationPrefs? prefs,
  bool subscribed = true,
}) async {
  final rig = await pumpApp(
    tester,
    rig: Rig(
      bridge: FakeBridge(
        state: subscribed ? idleState() : noSubscriptionState,
        caps: androidCaps,
      ),
      platform: android,
      vpn: FakeVpn(),
      autoAnswer: false,
    ),
  );
  final fake = permission ?? FakeNotificationPermission();
  final memory = prefs ?? MemoryNotificationPrefs();
  final guard = NotificationPermissionGuard(
    controller: rig.controller,
    permission: fake,
    prefs: memory,
  );
  addTearDown(guard.dispose);
  await guard.start();
  await tester.pump();
  return Bench(rig, fake, memory, guard);
}

void main() {
  testWidgets('нет разрешения, не спрашивали: системный запрос один раз', (
    tester,
  ) async {
    final bench = await androidWith(tester);
    expect(bench.permission.requests, 1);
    expect(bench.prefs.wasAsked, isTrue);
    expect(bench.guard.granted, isTrue);
    expect(bench.guard.missing, isFalse);
  });

  testWidgets('разрешение уже есть — запроса нет', (tester) async {
    final bench = await androidWith(
      tester,
      permission: FakeNotificationPermission(granted: true),
    );
    expect(bench.permission.requests, 0);
    expect(bench.guard.granted, isTrue);
  });

  testWidgets('отказали — подсказка остаётся, второй запрос не мучает', (
    tester,
  ) async {
    final bench = await androidWith(
      tester,
      permission: FakeNotificationPermission(grantOnRequest: false),
    );
    expect(bench.permission.requests, 1);
    expect(bench.guard.missing, isTrue);

    bench.rig.bridge.emit(next: connectedState());
    await tester.pump();
    expect(bench.permission.requests, 1);
  });

  testWidgets('спрашивали в прошлый запуск — подсказка без нового запроса', (
    tester,
  ) async {
    final bench = await androidWith(
      tester,
      prefs: MemoryNotificationPrefs(wasAsked: true),
    );
    expect(bench.permission.requests, 0);
    expect(bench.guard.missing, isTrue);
  });

  testWidgets('подписки ещё нет (онбординг) — не спрашиваем', (tester) async {
    final bench = await androidWith(tester, subscribed: false);
    expect(bench.permission.requests, 0);
    expect(
      bench.guard.missing,
      isTrue,
      reason: 'подсказка нужна, когда дойдёт',
    );

    // Ссылку импортировали — самое время спросить, до первого подключения.
    bench.rig.bridge.emit(next: idleState());
    await tester.pump();
    expect(bench.permission.requests, 1);
  });

  testWidgets('системный запрос упал — приложение живёт, подсказка есть', (
    tester,
  ) async {
    final bench = await androidWith(
      tester,
      permission: FakeNotificationPermission()
        ..requestError = StateError('нет'),
    );
    expect(bench.permission.requests, 1);
    expect(bench.guard.missing, isTrue);
  });

  testWidgets('вернулись из системных настроек — подсказка уходит', (
    tester,
  ) async {
    final bench = await androidWith(
      tester,
      permission: FakeNotificationPermission(grantOnRequest: false),
    );
    expect(bench.guard.missing, isTrue);

    bench.permission.granted = true;
    await bench.guard.recheck();
    expect(bench.guard.missing, isFalse);
  });

  testWidgets('«Открыть настройки» ведёт в настройки уведомлений Android', (
    tester,
  ) async {
    final bench = await androidWith(
      tester,
      permission: FakeNotificationPermission(grantOnRequest: false),
    );
    await bench.guard.openSettings();
    expect(bench.permission.settingsOpened, 1);
  });

  group('переключатель «Уведомлять о переключении»', () {
    testWidgets('включили без разрешения — подсказка внизу экрана', (
      tester,
    ) async {
      final bench = await androidWith(
        tester,
        permission: FakeNotificationPermission(grantOnRequest: false),
      );
      await bench.rig.controller.updateSettings(
        (s) => s.copyWith(notifySwitches: true),
      );
      await tester.pump();
      expect(bench.rig.notices, [hint]);
    });

    testWidgets('включили при разрешении — молчим', (tester) async {
      final bench = await androidWith(
        tester,
        permission: FakeNotificationPermission(granted: true),
      );
      await bench.rig.controller.updateSettings(
        (s) => s.copyWith(notifySwitches: true),
      );
      await tester.pump();
      expect(bench.rig.notices, isEmpty);
    });

    testWidgets('другие настройки подсказку не вызывают', (tester) async {
      final bench = await androidWith(
        tester,
        permission: FakeNotificationPermission(grantOnRequest: false),
      );
      await bench.rig.controller.updateSettings(
        (s) => s.copyWith(killSwitch: true, dnsMode: DnsMode.allViaTunnel),
      );
      await tester.pump();
      expect(bench.rig.notices, isEmpty);
    });
  });
}
