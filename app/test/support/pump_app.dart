/// Запуск приложения в виджет-тесте на подделках моста и сервисов.
library;

import 'package:braid/app.dart';
import 'package:braid/app_extensions.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'rig.dart';

/// Телефонный размер: тексты не обрезаются шрифтом тестов (Ahem широкий).
const phoneSize = Size(420, 900);

/// Поднимает [BraidApp] поверх [rig] и ждёт, пока всё осядет. Диалоги
/// показывает настоящий `EventHost` — отвечает тест, нажимая кнопки.
Future<Rig> pumpApp(
  WidgetTester tester, {
  Rig? rig,
  Size size = phoneSize,
  AppExtensions extensions = const AppExtensions(),
  bool settle = true,
}) async {
  tester.view.devicePixelRatio = 1.0;
  tester.view.physicalSize = size;
  addTearDown(tester.view.reset);
  final app = rig ?? Rig(autoAnswer: false);
  addTearDown(() async {
    // Сначала снимаем дерево: приложение отписывается от контроллера.
    await tester.pumpWidget(const SizedBox.shrink());
    await app.dispose();
  });
  await tester.pumpWidget(
    BraidApp(controller: app.controller, extensions: extensions),
  );
  if (settle) await tester.pumpAndSettle();
  return app;
}

/// Открыть второстепенный экран плиткой главного (`tile-rules`,
/// `tile-subscription`, `tile-settings`, …).
Future<void> openTile(WidgetTester tester, String key) async {
  final tile = find.byKey(Key(key));
  await reveal(tester, tile);
  await tester.tap(tile);
  await tester.pumpAndSettle();
}

/// Прокрутить так, чтобы элемент оказался на экране, и дождаться раскладки:
/// без паузы `tap` целится по старой раскладке. Список ленивый — элемент,
/// которого ещё нет в дереве, сначала докручивается.
Future<void> reveal(WidgetTester tester, Finder finder) async {
  if (finder.evaluate().isEmpty) {
    final page = find.descendant(
      of: find.byType(Scaffold).last,
      matching: find.byType(Scrollable),
    );
    await tester.scrollUntilVisible(finder, 300, scrollable: page.first);
  }
  await tester.ensureVisible(finder);
  await tester.pumpAndSettle();
}

/// «Назад» в AppBar. `pageBack()` ищет кнопку по платформе хоста (на macOS —
/// Cupertino), а у Material-экрана приложения она одна на всех.
Future<void> goBack(WidgetTester tester) async {
  await tester.tap(find.byType(BackButton));
  await tester.pumpAndSettle();
}
