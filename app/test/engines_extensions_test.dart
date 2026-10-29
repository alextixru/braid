// Подключение тикета 11 к приложению: `kAppExtensions` заполнен, плитки
// «Движки», «Статистика» (только админу), «О программе» ведут на свои экраны,
// а при готовности ядра поднимаются значок в меню-баре (macOS), вход VK и
// проверка разрешения на уведомления (Android).

import 'package:braid/app_extensions.dart';
import 'package:braid/bridge/braid_bridge.dart' show PlatformCaps;
import 'package:braid/models/commands.dart';
import 'package:braid/models/core_state.dart';
import 'package:braid/screens/engines/engine_services.dart';
import 'package:braid/services/platform_info.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'engines_support.dart';
import 'support/fake_bridge.dart';
import 'support/fakes.dart';
import 'support/pump_app.dart';
import 'support/rig.dart';
import 'support/states.dart';

const linux = PlatformInfo(TargetPlatform.linux);

class Wiring {
  Wiring(
    this.rig,
    this.tray,
    this.icons,
    this.window,
    this.permission,
    this.store,
  );

  final Rig rig;
  final FakeSystemTray tray;
  final FakeTrayIcons icons;
  final FakeWindowControl window;
  final FakeNotificationPermission permission;
  final MemoryVkStore store;
}

/// Приложение с настоящим `kAppExtensions` и подделками вместо родного слоя.
Future<Wiring> openWired(
  WidgetTester tester, {
  CoreState? state,
  PlatformInfo platform = macOS,
  MemoryVkStore? store,
  FakeSystemTray? tray,
  FakeNotificationPermission? permission,
}) async {
  final isAndroid = platform.isAndroid;
  final bridge = FakeBridge(
    state: state ?? idleState(),
    caps: isAndroid
        ? androidCaps
        : platform.isMacOS
        ? desktopCaps
        : const PlatformCaps(perApp: false, menuBar: false, qrScan: false),
  );
  final rig = Rig(
    bridge: bridge,
    platform: platform,
    vpn: isAndroid ? FakeVpn() : null,
    autoAnswer: false,
  );
  final fakes = Wiring(
    rig,
    tray ?? FakeSystemTray(),
    FakeTrayIcons(),
    FakeWindowControl(),
    permission ?? FakeNotificationPermission(),
    store ?? MemoryVkStore(),
  );
  EngineServices.install(
    EngineServices(
      controller: rig.controller,
      store: fakes.store,
      web: FakeWebLauncher(),
      statsFiles: FakeStatsFiles(),
      systemTray: fakes.tray,
      trayIcons: fakes.icons,
      window: fakes.window,
      notificationPermission: fakes.permission,
      notificationPrefs: MemoryNotificationPrefs(),
    ),
  );
  await pumpApp(tester, rig: rig, extensions: kAppExtensions);
  await tester.pump();
  return fakes;
}

void main() {
  test('все пять слотов тикета 11 заполнены', () {
    expect(kAppExtensions.engines, isNotNull);
    expect(kAppExtensions.stats, isNotNull);
    expect(kAppExtensions.about, isNotNull);
    expect(kAppExtensions.wrapApp, isNotNull);
    expect(kAppExtensions.onReady, isNotNull);
  });

  group('плитки главного экрана', () {
    testWidgets(
      '«Движки» и «О программе» есть у всех, «Статистики» у не админа нет',
      (tester) async {
        await openWired(tester);
        expect(find.byKey(const Key('tile-engines')), findsOneWidget);
        expect(find.byKey(const Key('tile-about')), findsOneWidget);
        expect(find.byKey(const Key('tile-stats')), findsNothing);
      },
    );

    testWidgets('у админа «Статистика» появляется', (tester) async {
      await openWired(tester, state: idleState(admin: true));
      expect(find.byKey(const Key('tile-stats')), findsOneWidget);
    });

    testWidgets('каждая плитка открывает свой экран', (tester) async {
      await openWired(tester, state: connectedState(admin: true));
      for (final (tile, title) in [
        ('tile-engines', 'Движки'),
        ('tile-stats', 'Статистика'),
        ('tile-about', 'О программе'),
      ]) {
        await openTile(tester, tile);
        await tester.pump(const Duration(milliseconds: 100));
        expect(
          find.widgetWithText(AppBar, title),
          findsOneWidget,
          reason: title,
        );
        await goBack(tester);
      }
    });
  });

  group('когда ядро готово', () {
    testWidgets('macOS: значок и меню в меню-баре поднимаются, окно готово', (
      tester,
    ) async {
      final wired = await openWired(tester);
      expect(wired.tray.shown, isTrue);
      expect(wired.window.prepared, isTrue);
      expect(wired.tray.icons, ['B:idle']);
      expect(
        wired.permission.requests,
        0,
        reason: 'уведомления — забота Android',
      );
    });

    testWidgets(
      'сохранённые хеши и вход VK возвращаются движку при его запросе',
      (tester) async {
        const hash = 'kw6VzMMQPO8fEtP0Yoo6_8nbuKfTCjxNq1GJdpQTnUA';
        final wired = await openWired(
          tester,
          store: MemoryVkStore(token: 'vk-token', hashes: [hash]),
        );
        wired.rig.bridge.emit(
          next: idleState(
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
        await tester.pump();
        await tester.pump(const Duration(seconds: 1));
        expect(wired.rig.bridge.userInputs, [
          ('csqtt', const UserInput.vkToken('vk-token')),
          ('csqtt', const UserInput.vkHashes([hash])),
        ]);
      },
    );

    testWidgets(
      'Android: разрешение на уведомления спрашивается, значка в меню-баре нет',
      (tester) async {
        final wired = await openWired(
          tester,
          platform: android,
          permission: FakeNotificationPermission(),
        );
        expect(wired.permission.requests, 1);
        expect(wired.tray.shown, isFalse);
        expect(wired.window.prepared, isFalse);
      },
    );

    testWidgets('Linux и Windows: меню-бара нет, значок не поднимается', (
      tester,
    ) async {
      final wired = await openWired(tester, platform: linux);
      expect(wired.tray.shown, isFalse);
      expect(wired.window.prepared, isFalse);
    });

    testWidgets('значок не поднялся — сообщение, остальное работает', (
      tester,
    ) async {
      final wired = await openWired(tester, tray: _BrokenTray());
      expect(
        wired.rig.notices.single,
        startsWith('Не удалось запустить значок в меню-баре'),
      );
      expect(find.byKey(const Key('tile-engines')), findsOneWidget);
    });
  });
}

class _BrokenTray extends FakeSystemTray {
  @override
  Future<void> show({required void Function(String id) onSelected}) =>
      Future.error(StateError('нет доступа к строке меню'));
}
