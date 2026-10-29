// Сводка журнала админа (ист. 63): переключения за 7 дней, доля времени по
// движкам, по сетям — где что падало. Журнал — JSONL ядра (`LogRecord` в
// `braid-proto`), считается так же, как `braidctl stats` (ист. 65): сессия
// движка в сети идёт от `connect`/`switch`/`network` до следующего события, в
// окно попадает только часть после среза.

import 'package:braid/screens/stats/stats_report.dart';
import 'package:flutter_test/flutter_test.dart';

final now = DateTime.utc(2026, 10, 29, 12);

/// Журнал, разобранный вручную. Срез окна — 22 окт 12:00.
///
///   awg   @ wifi:home: 6 ч (с среза до смены сети) + 30 мин = 6 ч 30 мин
///   awg   @ wifi:cafe: 6 ч + 1 ч                            = 7 ч
///   csqtt @ wifi:cafe: 5 ч
///   csqtt @ wifi:home: 1 ч (до последней записи)
///   всего awg 13 ч 30 мин, csqtt 6 ч → доли 13,5/19,5 и 6/19,5
const journal = '''
{"ts":"2026-10-21T10:00:00Z","event":"connect","engine":"awg","network":"wifi:home"}
{"ts":"2026-10-22T18:00:00Z","event":"network","id":"wifi:cafe","kind":"wifi"}
{"ts":"2026-10-23T00:00:00Z","event":"switch","from":"awg","to":"csqtt","reason":"dead","metrics":{"rtt_ms":null,"loss_pct":100,"handshake_age_s":200}}
{"ts":"2026-10-23T02:00:00Z","event":"probe","engine":"csqtt","rtt_ms":40,"lost":false}
{"ts":"2026-10-23T05:00:00Z","event":"switch","from":"csqtt","to":"awg","reason":"priority","metrics":{"rtt_ms":30,"loss_pct":0,"handshake_age_s":5}}
{"ts":"2026-10-23T06:00:00Z","event":"disconnect","manual":true}
{"ts":"2026-10-28T09:00:00Z","event":"connect","engine":"awg","network":"wifi:home"}
{"ts":"2026-10-28T09:30:00Z","event":"switch","from":"awg","to":"csqtt","reason":"degraded","metrics":{"rtt_ms":2300,"loss_pct":70,"handshake_age_s":20}}
{"ts":"2026-10-28T10:30:00Z","event":"probe","engine":"csqtt","rtt_ms":45,"lost":false}
''';

void main() {
  late StatsReport report;

  setUp(() => report = aggregateJournal(journal, now: now));

  test(
    'переключения за 7 дней: три, свежие сверху, с причиной и метриками',
    () {
      expect(report.switches, hasLength(3));
      final newest = report.switches.first;
      expect(newest.at, DateTime.utc(2026, 10, 28, 9, 30));
      expect(newest.from, 'awg');
      expect(newest.to, 'csqtt');
      expect(newest.reason, 'degraded');
      expect(newest.lossPct, 70);
      expect(newest.rttMs, 2300);
      expect(newest.network, 'wifi:home');
      expect(report.switches.map((s) => s.reason), [
        'degraded',
        'priority',
        'dead',
      ]);
    },
  );

  test('переключение старше окна не считается', () {
    final old = aggregateJournal(journal, now: DateTime.utc(2026, 11, 20));
    expect(old.switches, isEmpty);
    expect(old.uptime, isEmpty);
  });

  test('время по движкам — только часть сессий после среза', () {
    expect(report.uptime['awg'], const Duration(hours: 13, minutes: 30));
    expect(report.uptime['csqtt'], const Duration(hours: 6));
    expect(report.total, const Duration(hours: 19, minutes: 30));
  });

  test('доля времени по движкам', () {
    expect(report.share('awg'), closeTo(13.5 / 19.5, 1e-9));
    expect(report.share('csqtt'), closeTo(6 / 19.5, 1e-9));
    expect(report.share('нет такого'), 0);
  });

  test('по сетям: время каждого движка, переключения и откуда падало', () {
    final byName = {for (final n in report.networks) n.network: n};
    expect(byName.keys, unorderedEquals(['wifi:home', 'wifi:cafe']));

    final home = byName['wifi:home']!;
    expect(home.uptime['awg'], const Duration(hours: 6, minutes: 30));
    expect(home.uptime['csqtt'], const Duration(hours: 1));
    expect(home.switches, 1);
    expect(home.failures, {'awg': 1}, reason: 'degraded с awg — падение');
    expect(home.reasons, {'degraded': 1});

    final cafe = byName['wifi:cafe']!;
    expect(cafe.uptime['awg'], const Duration(hours: 7));
    expect(cafe.uptime['csqtt'], const Duration(hours: 5));
    expect(cafe.switches, 2);
    expect(cafe.failures, {
      'awg': 1,
    }, reason: 'dead с awg — падение, возврат по приоритету — нет');
    expect(cafe.reasons, {'dead': 1, 'priority': 1});
  });

  test('события в окне и битые строки считаются отдельно', () {
    expect(report.events, 8, reason: 'первая запись старше среза');
    final dirty = aggregateJournal(
      '$journal\nне json\n{"ts":"2026-10-28T11:00:00Z","event":"switch"}\n\n',
      now: now,
    );
    expect(dirty.badLines, 2, reason: 'пустая строка не в счёт');
    expect(dirty.switches, hasLength(3));
  });

  test('события будущих версий пропускаются, журнал не ломается', () {
    final future = aggregateJournal(
      '{"ts":"2026-10-28T11:00:00Z","event":"из_будущего","x":1}\n$journal',
      now: now,
    );
    expect(future.badLines, 0);
    expect(future.switches, hasLength(3));
  });

  test('пустой журнал — пустая сводка', () {
    final empty = aggregateJournal('', now: now);
    expect(empty.events, 0);
    expect(empty.switches, isEmpty);
    expect(empty.total, Duration.zero);
    expect(empty.share('awg'), 0);
  });
}
