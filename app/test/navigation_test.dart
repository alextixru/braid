// Навигация и точки подключения экранов тикета 11: плитки главного экрана,
// пункты «Движки / Статистика / О программе» только когда экран подключён
// (статистика — ещё и только админу), хуки `onReady` и `wrapApp`.

import 'package:braid/app_extensions.dart';
import 'package:braid/screens/home/home_screen.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'support/fake_bridge.dart';
import 'support/fakes.dart';
import 'support/pump_app.dart';
import 'support/rig.dart';
import 'support/states.dart';

Widget screen(String title) => Scaffold(
  appBar: AppBar(title: Text(title)),
  body: Text('Содержимое: $title'),
);

AppExtensions allScreens() => AppExtensions(
  engines: (_) => screen('Движки (экран)'),
  stats: (_) => screen('Статистика (экран)'),
  about: (_) => screen('О программе (экран)'),
);

void main() {
  testWidgets('плитки ведут на «Правила», «Подписка», «Настройки» и обратно', (
    tester,
  ) async {
    await pumpApp(tester);
    for (final (key, title) in [
      ('tile-rules', 'Правила'),
      ('tile-subscription', 'Подписка'),
      ('tile-settings', 'Настройки'),
    ]) {
      await openTile(tester, key);
      expect(find.widgetWithText(AppBar, title), findsOneWidget, reason: title);
      await goBack(tester);
      expect(find.byType(HomeScreen), findsOneWidget);
    }
  });

  testWidgets('пока экраны тикета 11 не подключены, пунктов для них нет', (
    tester,
  ) async {
    await pumpApp(
      tester,
      rig: Rig(
        bridge: FakeBridge(state: idleState(admin: true)),
        autoAnswer: false,
      ),
    );
    for (final key in ['tile-engines', 'tile-stats', 'tile-about']) {
      expect(find.byKey(Key(key)), findsNothing, reason: key);
    }
  });

  testWidgets('подключённые экраны появляются в списке и открываются', (
    tester,
  ) async {
    await pumpApp(
      tester,
      rig: Rig(
        bridge: FakeBridge(state: idleState(admin: true)),
        autoAnswer: false,
      ),
      extensions: allScreens(),
    );
    await openTile(tester, 'tile-engines');
    expect(find.text('Содержимое: Движки (экран)'), findsOneWidget);
    await goBack(tester);

    await openTile(tester, 'tile-stats');
    expect(find.text('Содержимое: Статистика (экран)'), findsOneWidget);
    await goBack(tester);

    await openTile(tester, 'tile-about');
    expect(find.text('Содержимое: О программе (экран)'), findsOneWidget);
  });

  testWidgets('«Статистика» видна только админу', (tester) async {
    await pumpApp(
      tester,
      rig: Rig(bridge: FakeBridge(state: idleState()), autoAnswer: false),
      extensions: allScreens(),
    );
    expect(find.byKey(const Key('tile-engines')), findsOneWidget);
    expect(find.byKey(const Key('tile-stats')), findsNothing);
  });

  testWidgets(
    '«Статистика» появляется, когда подписка делает пользователя админом',
    (tester) async {
      final rig = await pumpApp(
        tester,
        rig: Rig(bridge: FakeBridge(state: idleState()), autoAnswer: false),
        extensions: allScreens(),
      );
      rig.bridge.emit(next: idleState(admin: true));
      await tester.pumpAndSettle();
      expect(find.byKey(const Key('tile-stats')), findsOneWidget);
    },
  );

  testWidgets(
    'onReady вызывается один раз, когда ядро готово, и получает контроллер',
    (tester) async {
      final calls = <Object>[];
      final rig = await pumpApp(
        tester,
        extensions: AppExtensions(onReady: calls.add),
      );
      expect(calls, [rig.controller]);

      rig.bridge.emit(next: connectedState());
      await tester.pumpAndSettle();
      expect(calls, hasLength(1));
    },
  );

  testWidgets('onReady ждёт ядро: на Android это сервис', (tester) async {
    final calls = <Object>[];
    final bridge = FakeBridge(ready: false, caps: androidCaps);
    final rig = Rig(
      bridge: bridge,
      platform: android,
      vpn: FakeVpn(),
      autoAnswer: false,
    );
    await pumpApp(
      tester,
      rig: rig,
      extensions: AppExtensions(onReady: calls.add),
      settle: false,
    );
    await tester.pump(const Duration(milliseconds: 100));
    expect(calls, isEmpty);

    bridge.emit(isReady: true);
    await tester.pumpAndSettle();
    expect(calls, hasLength(1));
  });

  testWidgets(
    'wrapApp оборачивает всё приложение и видит навигатор для диалогов',
    (tester) async {
      await pumpApp(
        tester,
        extensions: AppExtensions(
          wrapApp: (context, app) => Stack(
            textDirection: TextDirection.ltr,
            children: [
              app,
              const Positioned(top: 0, left: 0, child: Text('надстройка')),
            ],
          ),
        ),
      );
      expect(find.text('надстройка'), findsOneWidget);
      expect(find.byType(HomeScreen), findsOneWidget);
    },
  );

  testWidgets('Android: возврат в приложение подбирает домен из share-меню', (
    tester,
  ) async {
    final vpn = FakeVpn();
    final rig = Rig(
      bridge: FakeBridge(caps: androidCaps),
      platform: android,
      vpn: vpn,
      autoAnswer: false,
    );
    await pumpApp(tester, rig: rig);

    vpn.sharedDomain = 'site.com';
    tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.inactive);
    tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.hidden);
    tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.paused);
    tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.hidden);
    tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.inactive);
    tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
    await tester.pumpAndSettle();

    expect(find.text('Добавить site.com?'), findsOneWidget);
  });

  testWidgets('тёмная тема: главный экран строится без ошибок', (tester) async {
    tester.platformDispatcher.platformBrightnessTestValue = Brightness.dark;
    addTearDown(tester.platformDispatcher.clearAllTestValues);
    await pumpApp(
      tester,
      rig: Rig(
        bridge: FakeBridge(state: connectedState(admin: true)),
        autoAnswer: false,
      ),
    );
    expect(find.byType(HomeScreen), findsOneWidget);
    final theme = Theme.of(tester.element(find.byType(HomeScreen)));
    expect(theme.brightness, Brightness.dark);
    expect(theme.useMaterial3, isTrue);
  });
}
