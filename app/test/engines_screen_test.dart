// Экран «Движки» (ист. 22, 30, 31, 74): каждый движок с живостью, закрепить
// или вернуть «Авто», «Проверить связь» с отчётом по всем движкам ≤ 10 с.

import 'dart:async';

import 'package:braid/app_extensions.dart';
import 'package:braid/bridge/braid_bridge.dart';
import 'package:braid/models/commands.dart';
import 'package:braid/models/core_state.dart';
import 'package:braid/models/results.dart';
import 'package:braid/screens/engines/engine_services.dart';
import 'package:braid/screens/engines/engines_screen.dart';
import 'package:braid/tray/notification_permission.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'engines_support.dart';
import 'support/fake_bridge.dart';
import 'support/fakes.dart';
import 'support/pump_app.dart';
import 'support/rig.dart';
import 'support/states.dart';

/// «Проверить связь», которая может зависнуть или отказать.
class ProbeBridge extends FakeBridge {
  ProbeBridge({super.state, super.caps});

  /// Пока не завершён, проба «идёт».
  Completer<List<EngineProbeReport>>? hang;
  BridgeError? failure;

  @override
  Future<List<EngineProbeReport>> probeNow() {
    final wait = hang;
    if (wait != null) return wait.future;
    final error = failure;
    if (error != null) return Future.error(error);
    return super.probeNow();
  }
}

Future<Rig> openEngines(
  WidgetTester tester, {
  CoreState? state,
  ProbeBridge? bridge,
  bool onAndroid = false,
  FakeNotificationPermission? permission,
}) async {
  final fake =
      (bridge ?? ProbeBridge(caps: onAndroid ? androidCaps : desktopCaps))
        ..state = state ?? connectedState();
  final rig = Rig(
    bridge: fake,
    platform: onAndroid ? android : macOS,
    vpn: onAndroid ? FakeVpn() : null,
    autoAnswer: false,
  );
  EngineServices.install(
    EngineServices(
      controller: rig.controller,
      store: MemoryVkStore(),
      web: FakeWebLauncher(),
      statsFiles: FakeStatsFiles(),
      notificationPermission:
          permission ?? FakeNotificationPermission(granted: true),
      notificationPrefs: MemoryNotificationPrefs(wasAsked: true),
    ),
  );
  await pumpApp(
    tester,
    rig: rig,
    extensions: const AppExtensions(engines: buildEnginesScreen),
  );
  await openTile(tester, 'tile-engines');
  return rig;
}

Finder inCard(String id, Finder what) =>
    find.descendant(of: find.byKey(Key('engine-$id')), matching: what);

const failed = EngineView(id: 'awg', kind: 'awg', state: EngineStatus.dead);

void main() {
  group('список', () {
    testWidgets('каждый движок: состояние, отклик, потери, хендшейк, пометки', (
      tester,
    ) async {
      await openEngines(tester);

      expect(find.widgetWithText(AppBar, 'Движки'), findsOneWidget);
      expect(inCard('awg', find.text('AWG')), findsOneWidget);
      expect(
        inCard('awg', find.text('Работает · несёт трафик')),
        findsOneWidget,
      );
      expect(
        inCard(
          'awg',
          find.text('Отклик 38 мс · потери 0 % · хендшейк 12 с назад'),
        ),
        findsOneWidget,
      );
      expect(inCard('awg', find.text('активный')), findsOneWidget);
      expect(inCard('awg', find.text('тёплый')), findsNothing);

      expect(inCard('csqtt', find.text('CSQTT')), findsOneWidget);
      expect(inCard('csqtt', find.text('Работает')), findsOneWidget);
      expect(inCard('csqtt', find.text('тёплый')), findsOneWidget);
    });

    testWidgets('«закреплён» виден; мёртвый закреплённый не подменяется', (
      tester,
    ) async {
      await openEngines(
        tester,
        state: idleState(
          session: const SessionConnected(),
          active: 'csqtt',
          engines: const [
            EngineView(
              id: 'awg',
              kind: 'awg',
              state: EngineStatus.dead,
              pinned: true,
            ),
            EngineView(
              id: 'csqtt',
              kind: 'csqtt',
              state: EngineStatus.connected,
            ),
          ],
        ),
      );
      expect(inCard('awg', find.text('закреплён')), findsOneWidget);
      expect(
        inCard(
          'awg',
          find.text('Недоступен — закреплён вручную, автопереключения нет'),
        ),
        findsOneWidget,
      );
      expect(find.text('Закреплён вручную: AWG'), findsOneWidget);
    });

    testWidgets('движок ждёт капчу, хеши или вход VK (WaitingUser)', (
      tester,
    ) async {
      await openEngines(
        tester,
        state: idleState(
          session: const SessionConnected(),
          active: 'awg',
          engines: const [
            EngineView(id: 'awg', kind: 'awg', state: EngineStatus.connected),
            EngineView(
              id: 'csqtt',
              kind: 'csqtt',
              state: EngineStatus.waitingUser,
              userAction: CaptchaAction(
                url: 'https://id.vk.com/x',
                session: 's',
              ),
            ),
          ],
        ),
      );
      expect(inCard('csqtt', find.text('Ждёт капчу')), findsOneWidget);
    });

    testWidgets('нужны новые хеши — статус на карточке', (tester) async {
      await openEngines(
        tester,
        state: idleState(
          session: const SessionConnected(),
          active: 'awg',
          engines: const [
            EngineView(id: 'awg', kind: 'awg', state: EngineStatus.connected),
            EngineView(
              id: 'csqtt',
              kind: 'csqtt',
              state: EngineStatus.waitingUser,
              userAction: NeedHashesAction(),
            ),
          ],
        ),
      );
      expect(inCard('csqtt', find.text('Нужны новые хеши')), findsOneWidget);
    });

    testWidgets('движок деградировал (Degraded) — это сказано', (tester) async {
      await openEngines(
        tester,
        state: idleState(
          session: const SessionConnected(),
          active: 'awg',
          engines: const [
            EngineView(
              id: 'awg',
              kind: 'awg',
              state: EngineStatus.degraded,
              health: Health(rttMs: 2300, lossPct: 70, handshakeAgeS: 20),
            ),
            EngineView(
              id: 'csqtt',
              kind: 'csqtt',
              warm: true,
              state: EngineStatus.connected,
            ),
          ],
        ),
      );
      expect(inCard('awg', find.text('Связь деградировала')), findsOneWidget);
      expect(
        inCard(
          'awg',
          find.text('Отклик 2300 мс · потери 70 % · хендшейк 20 с назад'),
        ),
        findsOneWidget,
      );
    });

    testWidgets('без подключения: движки остановлены, данных об отклике нет', (
      tester,
    ) async {
      await openEngines(tester, state: idleState());
      expect(inCard('awg', find.text('Остановлен')), findsOneWidget);
      expect(inCard('awg', find.textContaining('Отклик')), findsNothing);
    });
  });

  group('закрепить и «Авто»', () {
    testWidgets('«Закрепить» закрепляет движок, «Открепить» возвращает Авто', (
      tester,
    ) async {
      final rig = await openEngines(tester);
      expect(
        find.text('Авто — движок выбирается по приоритету'),
        findsOneWidget,
      );

      await tester.tap(find.byKey(const Key('engine-pin-csqtt')));
      await tester.pumpAndSettle();
      expect(rig.bridge.engineModes.last, const EngineMode.pinned('csqtt'));
      expect(inCard('csqtt', find.text('закреплён')), findsOneWidget);
      expect(find.text('Закреплён вручную: CSQTT'), findsOneWidget);

      await tester.tap(find.byKey(const Key('engine-pin-csqtt')));
      await tester.pumpAndSettle();
      expect(rig.bridge.engineModes.last, const EngineMode.auto());
      expect(inCard('csqtt', find.text('закреплён')), findsNothing);
    });

    testWidgets('«Вернуть Авто» над списком снимает любое закрепление', (
      tester,
    ) async {
      final rig = await openEngines(tester);
      await tester.tap(find.byKey(const Key('engine-pin-awg')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const Key('engines-auto')));
      await tester.pumpAndSettle();
      expect(rig.bridge.engineModes.last, const EngineMode.auto());
      expect(find.byKey(const Key('engines-auto')), findsNothing);
    });
  });

  group('«Проверить связь»', () {
    const reports = [
      EngineProbeReport(
        engine: 'awg',
        state: EngineStatus.connected,
        rttMs: 38,
        lossPct: 0,
        handshakeAgeS: 12,
      ),
      EngineProbeReport(
        engine: 'csqtt',
        state: EngineStatus.degraded,
        rttMs: 2100,
        lossPct: 55,
      ),
    ];

    testWidgets('отчёт по всем движкам: состояние, RTT, потери, хендшейк', (
      tester,
    ) async {
      final rig = await openEngines(
        tester,
        bridge: ProbeBridge()..probeReports = reports,
      );
      expect(find.byKey(const Key('probe-result')), findsNothing);

      await tester.tap(find.byKey(const Key('probe-button')));
      await tester.pumpAndSettle();

      expect(rig.bridge.calls, contains('probeNow'));
      expect(find.byKey(const Key('probe-result')), findsOneWidget);
      for (final (id, status, line) in [
        ('awg', 'Работает', 'Отклик 38 мс · потери 0 % · хендшейк 12 с назад'),
        ('csqtt', 'Связь деградировала', 'Отклик 2100 мс · потери 55 %'),
      ]) {
        final row = find.byKey(Key('probe-$id'));
        expect(
          find.descendant(of: row, matching: find.text(status)),
          findsOneWidget,
        );
        expect(
          find.descendant(of: row, matching: find.text(line)),
          findsOneWidget,
        );
      }
    });

    testWidgets('пока идёт проба — индикатор и кнопка занята', (tester) async {
      final bridge = ProbeBridge()..hang = Completer();
      await openEngines(tester, bridge: bridge);
      await tester.tap(find.byKey(const Key('probe-button')));
      await tester.pump();

      expect(find.text('Проверяю…'), findsOneWidget);
      final button = tester.widget<OutlinedButton>(
        find.byKey(const Key('probe-button')),
      );
      expect(button.onPressed, isNull);

      bridge.hang!.complete(reports);
      await tester.pumpAndSettle();
      expect(find.text('Проверяю…'), findsNothing);
      expect(find.byKey(const Key('probe-result')), findsOneWidget);
    });

    testWidgets('дольше 10 с не ждём: понятное сообщение, кнопка свободна', (
      tester,
    ) async {
      final bridge = ProbeBridge()..hang = Completer();
      await openEngines(tester, bridge: bridge);
      await tester.tap(find.byKey(const Key('probe-button')));
      await tester.pump();
      await tester.pump(const Duration(seconds: 9));
      expect(find.byKey(const Key('probe-error')), findsNothing);

      await tester.pump(const Duration(seconds: 2));
      expect(
        find.text('Проверка не уложилась в 10 с — связь с хостом нестабильна'),
        findsOneWidget,
      );
      final button = tester.widget<OutlinedButton>(
        find.byKey(const Key('probe-button')),
      );
      expect(button.onPressed, isNotNull);
    });

    testWidgets('ядро отказало — его текст на месте', (tester) async {
      await openEngines(
        tester,
        bridge: ProbeBridge()
          ..failure = const BridgeError(
            kind: BridgeErrorKind.notConnected,
            message: 'Сессия не активна',
            details: [],
          ),
      );
      await tester.tap(find.byKey(const Key('probe-button')));
      await tester.pumpAndSettle();
      expect(find.text('Сессия не активна'), findsOneWidget);
    });

    testWidgets('не подключено — проверять нечего, кнопка выключена', (
      tester,
    ) async {
      await openEngines(tester, state: idleState());
      final button = tester.widget<OutlinedButton>(
        find.byKey(const Key('probe-button')),
      );
      expect(button.onPressed, isNull);
      expect(find.text('Подключись, чтобы проверить связь'), findsOneWidget);
    });
  });

  group('уведомления (Android 13+)', () {
    testWidgets('разрешения нет — подсказка и кнопка в системные настройки', (
      tester,
    ) async {
      final permission = FakeNotificationPermission();
      await openEngines(tester, onAndroid: true, permission: permission);
      expect(find.byKey(const Key('notifications-hint')), findsOneWidget);
      expect(find.text(kNotificationHint), findsOneWidget);

      await tester.tap(find.byKey(const Key('notifications-open-settings')));
      await tester.pumpAndSettle();
      expect(permission.settingsOpened, 1);
    });

    testWidgets('разрешение есть — подсказки нет', (tester) async {
      await openEngines(
        tester,
        onAndroid: true,
        permission: FakeNotificationPermission(granted: true),
      );
      expect(find.byKey(const Key('notifications-hint')), findsNothing);
    });

    testWidgets('на macOS подсказки про уведомления нет', (tester) async {
      await openEngines(tester);
      expect(find.byKey(const Key('notifications-hint')), findsNothing);
    });
  });
}
