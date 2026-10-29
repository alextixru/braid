// Оболочка приложения: тема, локаль, корневой экран и его ветки — ожидание
// ядра, сбой запуска, онбординг без подписки, главный экран с подпиской.

import 'package:braid/app.dart';
import 'package:braid/bridge/braid_bridge.dart';
import 'package:braid/screens/home/home_screen.dart';
import 'package:braid/screens/onboarding/onboarding_screen.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'support/fake_bridge.dart';
import 'support/fakes.dart';
import 'support/pump_app.dart';
import 'support/rig.dart';
import 'support/states.dart';

void main() {
  testWidgets('Material 3, русская локаль, название одной константой', (
    tester,
  ) async {
    await pumpApp(tester);

    final app = tester.widget<MaterialApp>(find.byType(MaterialApp));
    expect(app.theme?.useMaterial3, isTrue);
    expect(app.darkTheme?.useMaterial3, isTrue);
    expect(app.title, appName);
    expect(appName, 'Braid');
    final context = tester.element(find.byType(Scaffold).first);
    expect(Localizations.localeOf(context).languageCode, 'ru');
  });

  testWidgets('без подписки — только онбординг', (tester) async {
    await pumpApp(
      tester,
      rig: Rig(
        bridge: FakeBridge(state: noSubscriptionState)..link = null,
        autoAnswer: false,
      ),
    );
    expect(find.byType(OnboardingScreen), findsOneWidget);
    expect(find.byType(HomeScreen), findsNothing);
  });

  testWidgets('с подпиской — главный экран', (tester) async {
    await pumpApp(tester);
    expect(find.byType(HomeScreen), findsOneWidget);
    expect(find.byType(OnboardingScreen), findsNothing);
  });

  testWidgets(
    'Android: пока сервис строит ядро, окно говорит «Сервис запускается»',
    (tester) async {
      final bridge = FakeBridge(ready: false, caps: androidCaps);
      final rig = Rig(
        bridge: bridge,
        platform: android,
        vpn: FakeVpn(),
        autoAnswer: false,
      );
      await pumpApp(tester, rig: rig, settle: false);
      await tester.pump(const Duration(milliseconds: 100));

      expect(find.byKey(const Key('splash-message')), findsOneWidget);
      expect(find.text('Сервис запускается…'), findsOneWidget);

      bridge.emit(isReady: true);
      await tester.pumpAndSettle();
      expect(find.byType(HomeScreen), findsOneWidget);
    },
  );

  testWidgets(
    'ядро не поднялось — причина и «Повторить», повтор доводит до экрана',
    (tester) async {
      final bridge = FakeBridge()
        ..startFailure = failure('Не открыть хранилище');
      await pumpApp(tester, rig: Rig(bridge: bridge, autoAnswer: false));

      expect(find.text('Не открыть хранилище'), findsOneWidget);
      expect(find.byType(HomeScreen), findsNothing);

      bridge.startFailure = null;
      await tester.tap(find.byKey(const Key('start-retry')));
      await tester.pumpAndSettle();
      expect(find.byType(HomeScreen), findsOneWidget);
    },
  );

  testWidgets('подписка пропала из ядра — снова онбординг, без перезапуска', (
    tester,
  ) async {
    final rig = await pumpApp(tester);
    expect(find.byType(HomeScreen), findsOneWidget);

    rig.bridge.emit(next: noSubscriptionState);
    await tester.pumpAndSettle();
    expect(find.byType(OnboardingScreen), findsOneWidget);
  });

  testWidgets(
    'другой контроллер в том же дереве запускается и показывает своё состояние',
    (tester) async {
      await pumpApp(
        tester,
        rig: Rig(
          bridge: FakeBridge(state: noSubscriptionState)..link = null,
          autoAnswer: false,
        ),
      );
      expect(find.byType(OnboardingScreen), findsOneWidget);

      await pumpApp(tester);
      expect(find.byType(HomeScreen), findsOneWidget);
      expect(find.byType(OnboardingScreen), findsNothing);
    },
  );

  testWidgets('библиотека ядра не загрузилась — окно объясняет, а не молчит', (
    tester,
  ) async {
    await tester.pumpWidget(
      const StartupFailedApp(message: 'Не найдена libbraid_ffi.dylib'),
    );
    expect(find.text('Braid не запустился'), findsOneWidget);
    expect(
      find.textContaining('Не найдена libbraid_ffi.dylib'),
      findsOneWidget,
    );
    final app = tester.widget<MaterialApp>(find.byType(MaterialApp));
    expect(app.theme?.useMaterial3, isTrue);
  });
}

BridgeError failure(String message) => BridgeError(
  kind: BridgeErrorKind.failed,
  message: message,
  details: const [],
);
