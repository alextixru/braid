// Первый запуск (ист. 1, 4, 35, 36): без подписки — один экран «Вставь ссылку
// или отсканируй QR»; подключиться нечем. Ошибка на месте, ввод не теряется;
// после импорта открывается главный экран; braid:// открывает приложение.

import 'dart:async';

import 'package:braid/models/results.dart';
import 'package:braid/screens/home/home_screen.dart';
import 'package:braid/screens/onboarding/onboarding_screen.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'support/fake_bridge.dart';
import 'support/fakes.dart';
import 'support/pump_app.dart';
import 'support/rig.dart';
import 'support/states.dart';

const _linkField = Key('link-field');

Rig emptyRig({
  FakeClipboard? clipboard,
  FakeDeepLinks? links,
  FakeQrScanner? scanner,
  bool qr = false,
}) {
  final bridge = FakeBridge(
    state: noSubscriptionState,
    caps: qr ? androidCaps : desktopCaps,
  )..link = null;
  return Rig(
    bridge: bridge,
    platform: qr ? android : macOS,
    vpn: qr ? FakeVpn() : null,
    clipboard: clipboard,
    links: links,
    scanner: scanner,
    autoAnswer: false,
  );
}

String fieldText(WidgetTester tester) =>
    tester.widget<TextField>(find.byKey(_linkField)).controller!.text;

void main() {
  testWidgets(
    'единственный экран: заголовок, поле, вставка, «Добавить»; подключиться нечем',
    (tester) async {
      await pumpApp(tester, rig: emptyRig());

      expect(find.byType(OnboardingScreen), findsOneWidget);
      expect(find.text('Вставь ссылку или отсканируй QR'), findsOneWidget);
      expect(find.byKey(_linkField), findsOneWidget);
      expect(find.text('Вставить из буфера'), findsOneWidget);
      expect(find.widgetWithText(FilledButton, 'Добавить'), findsOneWidget);

      // «Подключить» недоступна, и подсказка объясняет почему (ист. 4).
      final connect = tester.widget<FilledButton>(
        find.widgetWithText(FilledButton, 'Подключить'),
      );
      expect(connect.onPressed, isNull);
      expect(
        find.text('Подключиться можно, когда добавлена подписка'),
        findsOneWidget,
      );

      // Больше на экране ничего нет: ни правил, ни настроек.
      expect(find.text('Правила'), findsNothing);
      expect(find.text('Настройки'), findsNothing);
    },
  );

  testWidgets('на десктопе сканера QR нет', (tester) async {
    await pumpApp(tester, rig: emptyRig());
    expect(find.byKey(const Key('scan-button')), findsNothing);
  });

  testWidgets('вставка из буфера кладёт ссылку в поле', (tester) async {
    await pumpApp(
      tester,
      rig: emptyRig(clipboard: FakeClipboard('braid://abc')),
    );
    await tester.tap(find.byKey(const Key('paste-button')));
    await tester.pumpAndSettle();
    expect(fieldText(tester), 'braid://abc');
  });

  testWidgets('пустой буфер: подсказка, поле не меняется', (tester) async {
    await pumpApp(tester, rig: emptyRig(clipboard: FakeClipboard()));
    await tester.enterText(find.byKey(_linkField), 'что-то');
    await tester.tap(find.byKey(const Key('paste-button')));
    await tester.pumpAndSettle();
    expect(fieldText(tester), 'что-то');
    expect(find.text('В буфере пусто'), findsOneWidget);
  });

  testWidgets(
    '«Это не ссылка Braid» показывается на месте, а введённое остаётся',
    (tester) async {
      final rig = await pumpApp(tester, rig: emptyRig());

      await tester.enterText(find.byKey(_linkField), 'привет, это не ссылка');
      await tester.tap(find.byKey(const Key('add-button')));
      await tester.pumpAndSettle();

      expect(find.text('Это не ссылка Braid'), findsOneWidget);
      expect(fieldText(tester), 'привет, это не ссылка');
      expect(find.byType(OnboardingScreen), findsOneWidget);
      expect(rig.bridge.importedLinks, ['привет, это не ссылка']);
    },
  );

  testWidgets('ошибка исчезает, как только пользователь правит ввод', (
    tester,
  ) async {
    await pumpApp(tester, rig: emptyRig());
    await tester.enterText(find.byKey(_linkField), 'привет');
    await tester.tap(find.byKey(const Key('add-button')));
    await tester.pumpAndSettle();
    expect(find.text('Это не ссылка Braid'), findsOneWidget);

    await tester.enterText(find.byKey(_linkField), 'привет!');
    await tester.pump();
    expect(find.text('Это не ссылка Braid'), findsNothing);
  });

  testWidgets('пустое поле: «Добавить» просит ссылку и ядро не трогает', (
    tester,
  ) async {
    final rig = await pumpApp(tester, rig: emptyRig());
    await tester.tap(find.byKey(const Key('add-button')));
    await tester.pumpAndSettle();
    expect(
      find.text('Сначала вставь ссылку или отсканируй QR'),
      findsOneWidget,
    );
    expect(rig.bridge.importedLinks, isEmpty);
  });

  testWidgets('настоящая ссылка импортируется, открывается главный экран', (
    tester,
  ) async {
    final rig = await pumpApp(tester, rig: emptyRig());

    await tester.enterText(find.byKey(_linkField), '  braid://abc \n');
    await tester.tap(find.byKey(const Key('add-button')));
    await tester.pumpAndSettle();

    expect(rig.bridge.importedLinks, ['braid://abc']);
    expect(find.byType(HomeScreen), findsOneWidget);
    expect(find.byType(OnboardingScreen), findsNothing);
  });

  testWidgets('Enter в поле — то же, что «Добавить»', (tester) async {
    final rig = await pumpApp(tester, rig: emptyRig());
    await tester.enterText(find.byKey(_linkField), 'braid://abc');
    await tester.testTextInput.receiveAction(TextInputAction.done);
    await tester.pumpAndSettle();
    expect(rig.bridge.importedLinks, ['braid://abc']);
  });

  testWidgets(
    'пока ссылка загружается, кнопка занята и второй раз ссылку не шлёт',
    (tester) async {
      final rig = emptyRig();
      final gate = Completer<ImportOutcome>();
      rig.bridge.importHandler = (_) => gate.future;
      await pumpApp(tester, rig: rig);

      await tester.enterText(find.byKey(_linkField), 'braid://abc');
      await tester.tap(find.byKey(const Key('add-button')));
      await tester.pump();
      expect(find.byType(CircularProgressIndicator), findsOneWidget);
      await tester.tap(
        find.byKey(const Key('add-button')),
        warnIfMissed: false,
      );
      await tester.pump();
      expect(rig.bridge.importedLinks, ['braid://abc']);

      gate.complete(const Imported());
      await tester.pumpAndSettle();
      expect(find.byType(CircularProgressIndicator), findsNothing);
    },
  );

  testWidgets('Android: сканер QR — после кода ссылка импортируется сразу', (
    tester,
  ) async {
    final scanner = FakeQrScanner('braid://from-qr');
    final rig = await pumpApp(
      tester,
      rig: emptyRig(qr: true, scanner: scanner),
    );

    await tester.tap(find.byKey(const Key('scan-button')));
    await tester.pumpAndSettle();

    expect(scanner.opened, 1);
    expect(rig.bridge.importedLinks, ['braid://from-qr']);
    expect(find.byType(HomeScreen), findsOneWidget);
  });

  testWidgets('Android: QR не от Braid — ошибка на месте, прочитанное в поле', (
    tester,
  ) async {
    final scanner = FakeQrScanner('https://example.org/');
    await pumpApp(tester, rig: emptyRig(qr: true, scanner: scanner));

    await tester.tap(find.byKey(const Key('scan-button')));
    await tester.pumpAndSettle();

    expect(find.text('Это не ссылка Braid'), findsOneWidget);
    expect(fieldText(tester), 'https://example.org/');
  });

  testWidgets('Android: сканер закрыли без кода — ничего не меняется', (
    tester,
  ) async {
    final scanner = FakeQrScanner();
    final rig = await pumpApp(
      tester,
      rig: emptyRig(qr: true, scanner: scanner),
    );
    await tester.tap(find.byKey(const Key('scan-button')));
    await tester.pumpAndSettle();
    expect(rig.bridge.importedLinks, isEmpty);
    expect(find.byType(OnboardingScreen), findsOneWidget);
  });

  testWidgets(
    'braid:// из системы открывает приложение и сразу загружает подписку',
    (tester) async {
      final links = FakeDeepLinks();
      final rig = await pumpApp(tester, rig: emptyRig(links: links));
      expect(find.byType(OnboardingScreen), findsOneWidget);

      links.open('braid://from-deeplink');
      await tester.pumpAndSettle();

      expect(rig.bridge.importedLinks, ['braid://from-deeplink']);
      expect(find.byType(HomeScreen), findsOneWidget);
    },
  );

  testWidgets(
    'приложение, запущенное ссылкой, сразу показывает главный экран',
    (tester) async {
      final rig = await pumpApp(
        tester,
        rig: emptyRig(links: FakeDeepLinks(initial: 'braid://launch')),
      );
      expect(rig.bridge.importedLinks, ['braid://launch']);
      expect(find.byType(HomeScreen), findsOneWidget);
    },
  );
}
