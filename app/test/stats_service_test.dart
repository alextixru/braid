// Откуда экран «Статистика» берёт данные (ист. 63): единственный путь к
// журналу — `export_stats` (ядро склеивает дни в один файл и отдаёт путь),
// потому что каталог ядра на Android выбирает сервис, а не окно.

import 'dart:convert';
import 'dart:io';

import 'package:braid/bridge/braid_bridge.dart';
import 'package:braid/screens/stats/stats_service.dart';
import 'package:flutter_test/flutter_test.dart';

import 'engines_support.dart';
import 'support/fake_bridge.dart';
import 'support/rig.dart';
import 'support/states.dart';

const journal = '''
{"ts":"2026-10-28T09:00:00Z","event":"connect","engine":"awg","network":"wifi:home"}
{"ts":"2026-10-28T09:30:00Z","event":"switch","from":"awg","to":"csqtt","reason":"degraded","metrics":{"rtt_ms":2300,"loss_pct":70,"handshake_age_s":20}}
{"ts":"2026-10-28T10:30:00Z","event":"probe","engine":"csqtt","rtt_ms":45,"lost":false}
''';

class ExportBridge extends FakeBridge {
  ExportBridge({super.state});

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

Future<(StatsService, Rig, FakeStatsFiles)> serviceOver({
  ExportBridge? bridge,
  String text = journal,
}) async {
  final rig = Rig(
    bridge: bridge ?? ExportBridge(state: idleState(admin: true)),
    start: DateTime.utc(2026, 10, 29, 12),
    autoAnswer: false,
  );
  await rig.start();
  final files = FakeStatsFiles(text: text);
  addTearDown(rig.dispose);
  return (StatsService(controller: rig.controller, files: files), rig, files);
}

void main() {
  test('данные: export_stats → читаем файл по его пути → сводка', () async {
    final (service, rig, files) = await serviceOver();
    final report = await service.load();

    expect(rig.bridge.calls, contains('exportStats'));
    expect(files.read1, ['/tmp/braid-journal-export.jsonl']);
    expect(report.switches, hasLength(1));
    expect(report.switches.single.reason, 'degraded');
    expect(report.events, 3);
  });

  test('окно считается от «сейчас» приложения', () async {
    final (service, rig, _) = await serviceOver();
    expect((await service.load()).switches, hasLength(1));

    rig.clock = DateTime.utc(2026, 12, 1);
    final later = await service.load();
    expect(later.switches, isEmpty, reason: 'журнал старше 7 дней');
  });

  test(
    'ошибка ядра доходит до экрана как есть (не админ, журнал выключен)',
    () async {
      final bridge = ExportBridge(state: idleState(admin: true))
        ..exportError = const BridgeError(
          kind: BridgeErrorKind.adminOnly,
          message: 'Доступно только администратору',
          details: [],
        );
      final (service, _, files) = await serviceOver(bridge: bridge);
      await expectLater(
        service.load(),
        throwsA(
          isA<BridgeError>().having(
            (e) => e.message,
            'message',
            'Доступно только администратору',
          ),
        ),
      );
      expect(files.read1, isEmpty, reason: 'файла нет — читать нечего');
    },
  );

  test('экспорт: export_stats и предложение сохранить файл', () async {
    final (service, rig, files) = await serviceOver();
    final saved = await service.export();

    expect(rig.bridge.calls, contains('exportStats'));
    expect(files.offered, [
      ('/tmp/braid-journal-export.jsonl', 'braid-journal.jsonl'),
    ]);
    expect(saved, '/Users/me/braid-journal.jsonl');
  });

  test('отказ в диалоге сохранения — null, без ошибки', () async {
    final (service, _, files) = await serviceOver();
    files.savedTo = null;
    expect(await service.export(), isNull);
    expect(files.offered, hasLength(1), reason: 'файл предложили, он не нужен');
  });

  group('файлы на диске', () {
    late Directory dir;

    setUp(() => dir = Directory.systemTemp.createTempSync('braid-stats-test'));
    tearDown(() => dir.deleteSync(recursive: true));

    test('файл экспорта читается как UTF-8', () async {
      final file = File('${dir.path}/export.jsonl')
        ..writeAsStringSync('{"note":"сеть"}\n');
      expect(
        await const SystemStatsFiles().read(file.path),
        '{"note":"сеть"}\n',
      );
    });

    test('битый байт в журнале не роняет чтение', () async {
      final file = File('${dir.path}/export.jsonl')
        ..writeAsBytesSync([...utf8.encode('{"ok":1}\n'), 0xFF, 0x0A]);
      final text = await const SystemStatsFiles().read(file.path);
      expect(text, startsWith('{"ok":1}'));
    });
  });
}
