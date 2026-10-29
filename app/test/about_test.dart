// «О программе» (ист. 21, R34i): имя приложения одной константой, версии,
// компоненты и лицензии из `THIRD_PARTY.md`. Ограничение лицензии csqtt-core
// (только некоммерческое использование) видно сразу, а не в конце списка.

import 'package:braid/app.dart' show appName;
import 'package:braid/app_extensions.dart';
import 'package:braid/screens/about/about_screen.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

import 'support/fake_bridge.dart';
import 'support/pump_app.dart';
import 'support/rig.dart';

class BrokenBundle extends CachingAssetBundle {
  @override
  Future<ByteData> load(String key) =>
      Future.error(FlutterError('нет файла $key'));
}

Future<void> openAbout(WidgetTester tester, {AssetBundle? bundle}) async {
  final bridge = FakeBridge()
    ..client = '1.2.3'
    ..core = '4.5.6';
  await pumpApp(
    tester,
    rig: Rig(bridge: bridge, autoAnswer: false),
    extensions: AppExtensions(
      about: buildAboutScreen,
      wrapApp: bundle == null
          ? null
          : (context, app) => DefaultAssetBundle(bundle: bundle, child: app),
    ),
  );
  await openTile(tester, 'tile-about');
  await waitForComponents(tester);
}

/// Файл читается настоящим вводом-выводом, а виджет-тест живёт на поддельных
/// часах: даём реальное время, пока список компонентов (или сообщение об
/// ошибке) не появится.
Future<void> waitForComponents(WidgetTester tester) async {
  final ready = find.byKey(const Key('about-components'));
  final failed = find.text('Не удалось прочитать список компонентов');
  for (var i = 0; i < 40; i++) {
    if (ready.evaluate().isNotEmpty || failed.evaluate().isNotEmpty) break;
    await tester.runAsync(
      () => Future<void>.delayed(const Duration(milliseconds: 50)),
    );
    await tester.pump();
  }
  await tester.pumpAndSettle();
}

void main() {
  testWidgets('плитка ведёт на «О программе»', (tester) async {
    await openAbout(tester);
    expect(find.widgetWithText(AppBar, 'О программе'), findsOneWidget);
  });

  testWidgets('имя — из константы приложения, версии приложения и ядра', (
    tester,
  ) async {
    await openAbout(tester);
    expect(find.byKey(const Key('about-name')), findsOneWidget);
    expect(
      tester.widget<Text>(find.byKey(const Key('about-name'))).data,
      appName,
    );
    expect(find.text('Версия 1.2.3'), findsOneWidget);
    expect(find.text('Ядро 4.5.6'), findsOneWidget);
  });

  testWidgets('компоненты и лицензии читаются из THIRD_PARTY.md', (
    tester,
  ) async {
    await openAbout(tester);

    expect(find.textContaining('Движки и платформенный код'), findsOneWidget);
    await reveal(tester, find.textContaining('csqtt-core 2.1.9').first);
    expect(find.textContaining('csqtt-core 2.1.9'), findsWidgets);
    expect(find.text('PolyForm Noncommercial 1.0.0'), findsWidgets);

    final packages = find.textContaining('flutter_secure_storage');
    await reveal(tester, packages.first);
    expect(packages, findsWidgets);
    expect(find.textContaining('Flutter и пакеты Dart'), findsOneWidget);
  });

  testWidgets('ограничение csqtt-core — наверху, словами из самого файла', (
    tester,
  ) async {
    await openAbout(tester);
    final notice = find.byKey(const Key('about-noncommercial'));
    expect(notice, findsOneWidget);
    expect(
      find.descendant(
        of: notice,
        matching: find.text('Лицензия PolyForm Noncommercial 1.0.0'),
      ),
      findsOneWidget,
    );
    expect(
      find.descendant(
        of: notice,
        matching: find.textContaining('только некоммерческое использование'),
      ),
      findsOneWidget,
    );
  });

  testWidgets('файла нет: сообщение, а имя и версия на месте', (tester) async {
    await openAbout(tester, bundle: BrokenBundle());
    expect(
      find.text('Не удалось прочитать список компонентов'),
      findsOneWidget,
    );
    expect(find.text('Версия 1.2.3'), findsOneWidget);
    expect(find.byKey(const Key('about-noncommercial')), findsNothing);
  });

  testWidgets('полные тексты лицензий пакетов — стандартная страница Flutter', (
    tester,
  ) async {
    await openAbout(tester);
    await reveal(tester, find.byKey(const Key('about-licenses')));
    await tester.tap(find.byKey(const Key('about-licenses')));
    await tester.pumpAndSettle();
    expect(find.byType(LicensePage), findsOneWidget);
  });
}
