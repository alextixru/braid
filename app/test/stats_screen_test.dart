// Экран «Статистика» (ист. 63, A03): только админу — переключения за 7 дней,
// доля времени по движкам, по сетям, экспорт файла. У остальных ни экрана, ни
// журнала, ни вызова ядра (ист. 64).

import 'package:braid/app_extensions.dart';
import 'package:braid/bridge/braid_bridge.dart';
import 'package:braid/screens/engines/engine_services.dart';
import 'package:braid/screens/stats/stats_screen.dart';
import 'package:braid/util/format.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'engines_support.dart';
import 'support/fake_bridge.dart';
import 'support/pump_app.dart';
import 'support/rig.dart';
import 'support/states.dart';

class StatsBridge extends FakeBridge {
  StatsBridge({super.state});

  BridgeError? exportError;

  @override
  Future<String> exportStats() async {
    final error = exportError;
    if (error != null) {
      await super.exportStats();
      throw error;
    }
    return super.exportStats();
  }
}

class Ctx {
  Ctx(this.rig, this.bridge, this.files);

  final Rig rig;
  final StatsBridge bridge;
  final FakeStatsFiles files;
}

Future<Ctx> openApp(
  WidgetTester tester, {
  bool admin = true,
  String journal = statsJournal,
  StatsBridge? bridge,
}) async {
  final fake = bridge ?? StatsBridge();
  fake.state = connectedState(admin: admin);
  final rig = Rig(
    bridge: fake,
    start: DateTime.utc(2026, 10, 29, 12),
    autoAnswer: false,
  );
  final files = FakeStatsFiles(text: journal);
  EngineServices.install(
    EngineServices(
      controller: rig.controller,
      store: MemoryVkStore(),
      web: FakeWebLauncher(),
      statsFiles: files,
    ),
  );
  await pumpApp(
    tester,
    rig: rig,
    extensions: const AppExtensions(stats: buildStatsScreen),
  );
  return Ctx(rig, fake, files);
}

Future<void> openStats(WidgetTester tester) async {
  await openTile(tester, 'tile-stats');
  await tester.pumpAndSettle();
}

void main() {
  group('виден только админу', () {
    testWidgets('не админ: пункта нет, ядро за журналом не ходит', (
      tester,
    ) async {
      final ctx = await openApp(tester, admin: false);
      expect(find.byKey(const Key('tile-stats')), findsNothing);
      expect(ctx.bridge.calls, isNot(contains('exportStats')));
      expect(ctx.files.read1, isEmpty);
    });

    testWidgets('админ: пункт есть и ведёт на экран', (tester) async {
      await openApp(tester);
      await openStats(tester);
      expect(find.widgetWithText(AppBar, 'Статистика'), findsOneWidget);
    });

    testWidgets('права сняли, пока экран открыт — данные уходят с экрана', (
      tester,
    ) async {
      final ctx = await openApp(tester);
      await openStats(tester);
      expect(find.text('Переключений: 3'), findsOneWidget);

      ctx.rig.bridge.emit(next: connectedState(admin: false));
      await tester.pumpAndSettle();
      expect(
        find.text('Статистика доступна только администратору'),
        findsOneWidget,
      );
      expect(find.text('Переключений: 3'), findsNothing);
      expect(find.byKey(const Key('stats-export')), findsNothing);
    });
  });

  group('сводка за 7 дней', () {
    testWidgets('сколько переключений и событий', (tester) async {
      await openApp(tester);
      await openStats(tester);
      expect(find.text('За 7 дней'), findsOneWidget);
      expect(find.text('Переключений: 3'), findsOneWidget);
      expect(find.text('Событий в журнале: 8'), findsOneWidget);
    });

    testWidgets('доля времени по движкам', (tester) async {
      await openApp(tester);
      await openStats(tester);

      final awg = find.byKey(const Key('stats-engine-awg'));
      expect(
        find.descendant(of: awg, matching: find.text('AWG')),
        findsOneWidget,
      );
      expect(
        find.descendant(of: awg, matching: find.text('69 %')),
        findsOneWidget,
      );
      expect(
        find.descendant(of: awg, matching: find.text('13 ч 30 мин')),
        findsOneWidget,
      );

      final csqtt = find.byKey(const Key('stats-engine-csqtt'));
      expect(
        find.descendant(of: csqtt, matching: find.text('31 %')),
        findsOneWidget,
      );
      expect(
        find.descendant(of: csqtt, matching: find.text('6 ч')),
        findsOneWidget,
      );
      final bar = tester.widget<LinearProgressIndicator>(
        find.descendant(
          of: awg,
          matching: find.byType(LinearProgressIndicator),
        ),
      );
      expect(bar.value, closeTo(13.5 / 19.5, 1e-9));
    });

    testWidgets('по сетям: время, переключения и где что падало', (
      tester,
    ) async {
      await openApp(tester);
      await openStats(tester);

      final home = find.byKey(const Key('stats-network-wifi:home'));
      await reveal(tester, home);
      expect(
        find.descendant(
          of: home,
          matching: find.text(networkLabel('wifi:home')),
        ),
        findsOneWidget,
      );
      expect(
        find.descendant(
          of: home,
          matching: find.text('Время: AWG 6 ч 30 мин · CSQTT 1 ч'),
        ),
        findsOneWidget,
      );
      expect(
        find.descendant(of: home, matching: find.text('Падал: AWG — 1 раз')),
        findsOneWidget,
      );
      expect(
        find.descendant(
          of: home,
          matching: find.text('Причины: деградация — 1'),
        ),
        findsOneWidget,
      );

      final cafe = find.byKey(const Key('stats-network-wifi:cafe'));
      await reveal(tester, cafe);
      expect(
        find.descendant(
          of: cafe,
          matching: find.text(
            'Причины: возврат на приоритетный — 1, нет ответа — 1',
          ),
        ),
        findsOneWidget,
      );
      expect(
        find.descendant(of: cafe, matching: find.text('Падал: AWG — 1 раз')),
        findsOneWidget,
      );
    });

    testWidgets('переключения: свежие сверху, с причиной и метриками', (
      tester,
    ) async {
      await openApp(tester);
      await openStats(tester);

      final newest = find.byKey(const Key('stats-switch-0'));
      await reveal(tester, newest);
      expect(
        find.descendant(of: newest, matching: find.text('AWG → CSQTT')),
        findsOneWidget,
      );
      expect(
        find.descendant(
          of: newest,
          matching: find.textContaining('потери 70 %'),
        ),
        findsOneWidget,
      );
      expect(
        find.descendant(
          of: newest,
          matching: find.textContaining('деградация'),
        ),
        findsOneWidget,
      );
      final oldest = find.byKey(const Key('stats-switch-2'));
      await reveal(tester, oldest);
      expect(
        find.descendant(
          of: oldest,
          matching: find.textContaining('нет ответа'),
        ),
        findsOneWidget,
      );
    });

    testWidgets('в журнале пусто — так и сказано', (tester) async {
      await openApp(tester, journal: '');
      await openStats(tester);
      expect(find.text('За 7 дней событий нет'), findsOneWidget);
      expect(find.byKey(const Key('stats-engine-awg')), findsNothing);
    });

    testWidgets('«Обновить» перечитывает журнал', (tester) async {
      final ctx = await openApp(tester);
      await openStats(tester);
      final before = ctx.files.read1.length;
      await tester.tap(find.byKey(const Key('stats-refresh')));
      await tester.pumpAndSettle();
      expect(ctx.files.read1.length, before + 1);
    });
  });

  group('ошибки', () {
    testWidgets('ядро отказало — его текст и «Повторить»', (tester) async {
      final bridge = StatsBridge()
        ..exportError = const BridgeError(
          kind: BridgeErrorKind.failed,
          message: 'Ошибка ввода-вывода: диск',
          details: [],
        );
      final ctx = await openApp(tester, bridge: bridge);
      await openStats(tester);
      expect(find.text('Ошибка ввода-вывода: диск'), findsOneWidget);

      bridge.exportError = null;
      await tester.tap(find.byKey(const Key('stats-retry')));
      await tester.pumpAndSettle();
      expect(find.text('Переключений: 3'), findsOneWidget);
      expect(ctx.files.read1, isNotEmpty);
    });
  });

  group('экспорт', () {
    testWidgets('«Экспорт» зовёт export_stats и предлагает файл', (
      tester,
    ) async {
      final ctx = await openApp(tester);
      await openStats(tester);
      ctx.bridge.calls.clear();

      await reveal(tester, find.byKey(const Key('stats-export')));
      await tester.tap(find.byKey(const Key('stats-export')));
      await tester.pumpAndSettle();

      expect(ctx.bridge.calls, contains('exportStats'));
      expect(ctx.files.offered, [
        ('/tmp/braid-journal-export.jsonl', 'braid-journal.jsonl'),
      ]);
      expect(
        find.text('Сохранено: /Users/me/braid-journal.jsonl'),
        findsOneWidget,
      );
    });

    testWidgets('отказ в диалоге сохранения — молча', (tester) async {
      final ctx = await openApp(tester);
      await openStats(tester);
      ctx.files.savedTo = null;

      await reveal(tester, find.byKey(const Key('stats-export')));
      await tester.tap(find.byKey(const Key('stats-export')));
      await tester.pumpAndSettle();
      expect(ctx.files.offered, hasLength(1));
      expect(find.textContaining('Сохранено'), findsNothing);
    });
  });
}
