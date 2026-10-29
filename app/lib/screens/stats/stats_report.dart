/// Сводка журнала админа для экрана «Статистика» (ист. 63).
///
/// Журнал — JSONL ядра (`LogRecord` в `braid-proto`, ротация 30 дней), ядро
/// склеивает дни в один файл по `export_stats`. Считаем так же, как
/// `braidctl stats` (ист. 65), но для одного устройства: сессия движка в сети
/// идёт от `connect` / `switch` / `network` до следующего события, открытая —
/// до последней записи; в окно попадает только часть после среза. Сети в
/// журнале — хеши, доменов там нет.
library;

import 'dart:convert';

/// Падение движка: переключение с него по одной из этих причин. Возврат по
/// приоритету, ручное и «память сети» — не падения.
const failureReasons = {'dead', 'degraded', 'stall'};

/// Одно переключение движка.
class SwitchEntry {
  const SwitchEntry({
    required this.at,
    required this.from,
    required this.to,
    required this.reason,
    this.network,
    this.rttMs,
    this.lossPct = 0,
  });

  final DateTime at;
  final String from;
  final String to;

  /// Код причины ядра: `dead`, `degraded`, `stall`, `manual`, `priority`,
  /// `memory`, `startup`.
  final String reason;

  /// Сеть, в которой это произошло.
  final String? network;
  final int? rttMs;
  final int lossPct;
}

/// Что было в одной сети.
class NetworkStats {
  const NetworkStats({
    required this.network,
    required this.uptime,
    required this.switches,
    required this.failures,
    required this.reasons,
  });

  final String network;

  /// Время каждого движка в этой сети.
  final Map<String, Duration> uptime;

  /// Переключений с движков в этой сети.
  final int switches;

  /// Падения по движкам: сколько раз переключались с него из-за беды.
  final Map<String, int> failures;

  /// Переключения по причинам.
  final Map<String, int> reasons;

  Duration get total => uptime.values.fold(Duration.zero, (a, b) => a + b);
}

class StatsReport {
  const StatsReport({
    this.events = 0,
    this.badLines = 0,
    this.switches = const [],
    this.uptime = const {},
    this.networks = const [],
  });

  /// События журнала в окне.
  final int events;

  /// Строки, которые не разобрались.
  final int badLines;

  /// Переключения в окне, свежие сверху.
  final List<SwitchEntry> switches;

  /// Время по движкам.
  final Map<String, Duration> uptime;

  /// Сети, где было время или переключения; сначала самые долгие.
  final List<NetworkStats> networks;

  Duration get total => uptime.values.fold(Duration.zero, (a, b) => a + b);

  /// Доля времени движка от 0 до 1.
  double share(String engine) {
    final all = total.inMicroseconds;
    if (all == 0) return 0;
    return (uptime[engine]?.inMicroseconds ?? 0) / all;
  }
}

const _unknownNetwork = '?';

/// Свести [jsonl] за [window] до [now].
StatsReport aggregateJournal(
  String jsonl, {
  required DateTime now,
  Duration window = const Duration(days: 7),
}) {
  final cutoff = now.toUtc().subtract(window);
  var bad = 0;
  final records = <_Record>[];
  for (final line in const LineSplitter().convert(jsonl)) {
    final text = line.trim();
    if (text.isEmpty) continue;
    final record = _Record.tryParse(text, records.length);
    if (record == null) {
      bad++;
    } else {
      records.add(record);
    }
  }
  // Порядок событий с одной секундой сохраняется: сортировка не стабильна.
  records.sort((a, b) {
    final byTime = a.at.compareTo(b.at);
    return byTime != 0 ? byTime : a.index.compareTo(b.index);
  });

  final cells = <String, Map<String, Duration>>{};
  final switchCounts = <String, int>{};
  final failures = <String, Map<String, int>>{};
  final reasons = <String, Map<String, int>>{};
  final switches = <SwitchEntry>[];
  var events = 0;

  String? engine;
  String? network;
  DateTime? since;

  void accrue(DateTime until) {
    final current = engine;
    final startedAt = since;
    if (current == null || startedAt == null) return;
    final start = startedAt.isAfter(cutoff) ? startedAt : cutoff;
    if (!until.isAfter(start)) return;
    final perEngine = cells.putIfAbsent(network ?? _unknownNetwork, () => {});
    perEngine[current] =
        (perEngine[current] ?? Duration.zero) + until.difference(start);
  }

  for (final record in records) {
    if (!record.at.isBefore(cutoff)) events++;
    switch (record.kind) {
      case 'connect':
        accrue(record.at);
        network = record.network ?? network;
        engine = record.engine;
        since = record.at;
      case 'switch':
        accrue(record.at);
        if (!record.at.isBefore(cutoff)) {
          final where = network ?? _unknownNetwork;
          final from = record.from!;
          final reason = record.reason!;
          switchCounts[where] = (switchCounts[where] ?? 0) + 1;
          final byReason = reasons.putIfAbsent(where, () => {});
          byReason[reason] = (byReason[reason] ?? 0) + 1;
          if (failureReasons.contains(reason)) {
            final byEngine = failures.putIfAbsent(where, () => {});
            byEngine[from] = (byEngine[from] ?? 0) + 1;
          }
          switches.add(
            SwitchEntry(
              at: record.at,
              from: from,
              to: record.to!,
              reason: reason,
              network: network,
              rttMs: record.rttMs,
              lossPct: record.lossPct,
            ),
          );
        }
        engine = record.to;
        since = record.at;
      case 'disconnect':
        accrue(record.at);
        engine = null;
        since = null;
      case 'network':
        accrue(record.at);
        network = record.network;
        if (engine != null) since = record.at;
    }
  }
  if (records.isNotEmpty) accrue(records.last.at);

  final names = {...cells.keys, ...switchCounts.keys};
  final networks =
      [
        for (final name in names)
          NetworkStats(
            network: name,
            uptime: _byTimeDesc(cells[name] ?? const {}),
            switches: switchCounts[name] ?? 0,
            failures: Map.unmodifiable(failures[name] ?? const {}),
            reasons: Map.unmodifiable(reasons[name] ?? const {}),
          ),
      ]..sort((a, b) {
        final byTime = b.total.compareTo(a.total);
        return byTime != 0 ? byTime : a.network.compareTo(b.network);
      });

  final byEngine = <String, Duration>{};
  for (final network in networks) {
    network.uptime.forEach((engine, time) {
      byEngine[engine] = (byEngine[engine] ?? Duration.zero) + time;
    });
  }
  switches.sort((a, b) => b.at.compareTo(a.at));
  return StatsReport(
    events: events,
    badLines: bad,
    switches: List.unmodifiable(switches),
    uptime: _byTimeDesc(byEngine),
    networks: List.unmodifiable(networks),
  );
}

Map<String, Duration> _byTimeDesc(Map<String, Duration> times) {
  final entries = times.entries.toList()
    ..sort((a, b) {
      final byTime = b.value.compareTo(a.value);
      return byTime != 0 ? byTime : a.key.compareTo(b.key);
    });
  return Map.unmodifiable({for (final e in entries) e.key: e.value});
}

/// Одна строка журнала. Событие неизвестного вида (из будущей версии ядра)
/// остаётся записью без полей: оно считается в «событиях» и ничего не меняет.
class _Record {
  _Record._({
    required this.at,
    required this.index,
    required this.kind,
    this.engine,
    this.network,
    this.from,
    this.to,
    this.reason,
    this.rttMs,
    this.lossPct = 0,
  });

  final DateTime at;
  final int index;
  final String kind;
  final String? engine;
  final String? network;
  final String? from;
  final String? to;
  final String? reason;
  final int? rttMs;
  final int lossPct;

  static _Record? tryParse(String line, int index) {
    final Object? decoded;
    try {
      decoded = jsonDecode(line);
    } on FormatException {
      return null;
    }
    if (decoded is! Map<String, dynamic>) return null;
    final Map<String, dynamic> json = decoded;
    final stamp = json['ts'];
    final at = stamp is String ? DateTime.tryParse(stamp) : null;
    final kind = json['event'];
    if (at == null || kind is! String) return null;
    String? text(String key) {
      final value = json[key];
      return value is String ? value : null;
    }

    switch (kind) {
      case 'connect':
        final engine = text('engine');
        if (engine == null) return null;
        return _Record._(
          at: at.toUtc(),
          index: index,
          kind: kind,
          engine: engine,
          network: text('network'),
        );
      case 'switch':
        final from = text('from');
        final to = text('to');
        final reason = text('reason');
        if (from == null || to == null || reason == null) return null;
        final metrics = json['metrics'];
        final rtt = metrics is Map ? metrics['rtt_ms'] : null;
        final loss = metrics is Map ? metrics['loss_pct'] : null;
        return _Record._(
          at: at.toUtc(),
          index: index,
          kind: kind,
          from: from,
          to: to,
          reason: reason,
          rttMs: rtt is int ? rtt : null,
          lossPct: loss is int ? loss : 0,
        );
      case 'network':
        final id = text('id');
        if (id == null) return null;
        return _Record._(at: at.toUtc(), index: index, kind: kind, network: id);
      default:
        return _Record._(at: at.toUtc(), index: index, kind: kind);
    }
  }
}
