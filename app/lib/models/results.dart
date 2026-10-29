/// Ответы ядра на команды: каталог правил, итоги импорта и обновления,
/// отчёты «Проверить связь».
library;

import 'package:braid/models/core_state.dart';
import 'package:braid/models/json.dart';
import 'package:braid/util/value.dart';

/// Наборы и режимы подписки с исключениями — то, чего нет в `RulesView`.
class RulesCatalog with Value {
  const RulesCatalog({this.sets = const [], this.modes = const []});

  final List<SetEntry> sets;
  final List<ModeEntry> modes;

  factory RulesCatalog.fromJson(Map<String, dynamic> json) {
    return RulesCatalog(
      sets: List.unmodifiable(
        asList(
          json['sets'],
          'sets',
        ).map((s) => SetEntry.fromJson(asObject(s, 'sets[]'))),
      ),
      modes: List.unmodifiable(
        asList(
          json['modes'],
          'modes',
        ).map((m) => ModeEntry.fromJson(asObject(m, 'modes[]'))),
      ),
    );
  }

  SetEntry? setById(String id) {
    for (final set in sets) {
      if (set.id == id) return set;
    }
    return null;
  }

  ModeEntry? modeById(String? id) {
    if (id == null) return null;
    for (final mode in modes) {
      if (mode.id == id) return mode;
    }
    return null;
  }

  @override
  List<Object?> get props => [sets, modes];
}

class SetEntry with Value {
  const SetEntry({
    required this.id,
    required this.name,
    required this.domains,
    required this.ips,
    this.excludeDomains = const [],
    this.excludeIps = const [],
  });

  final String id;
  final String name;
  final int domains;
  final int ips;

  /// Никогда не идут в туннель, даже если попадают под правила набора.
  final List<String> excludeDomains;
  final List<String> excludeIps;

  factory SetEntry.fromJson(Map<String, dynamic> json) {
    return SetEntry(
      id: reqString(json, 'id'),
      name: reqString(json, 'name'),
      domains: reqInt(json, 'domains'),
      ips: reqInt(json, 'ips'),
      excludeDomains: reqStrings(json, 'exclude_domains'),
      excludeIps: reqStrings(json, 'exclude_ips'),
    );
  }

  @override
  List<Object?> get props => [
    id,
    name,
    domains,
    ips,
    excludeDomains,
    excludeIps,
  ];
}

class ModeEntry with Value {
  const ModeEntry({
    required this.id,
    required this.name,
    required this.sets,
    this.excludeDomains = const [],
    this.excludeIps = const [],
    this.isDefault = false,
  });

  final String id;
  final String name;

  /// `id` наборов, которые включает режим.
  final List<String> sets;
  final List<String> excludeDomains;
  final List<String> excludeIps;

  /// Включается сам после импорта подписки.
  final bool isDefault;

  factory ModeEntry.fromJson(Map<String, dynamic> json) {
    return ModeEntry(
      id: reqString(json, 'id'),
      name: reqString(json, 'name'),
      sets: reqStrings(json, 'sets'),
      excludeDomains: reqStrings(json, 'exclude_domains'),
      excludeIps: reqStrings(json, 'exclude_ips'),
      isDefault: reqBool(json, 'default'),
    );
  }

  @override
  List<Object?> get props => [
    id,
    name,
    sets,
    excludeDomains,
    excludeIps,
    isDefault,
  ];
}

/// Итог импорта ссылки: ссылка другого хоста при живой подписке — не ошибка,
/// а вопрос «заменить?».
sealed class ImportOutcome with Value {
  const ImportOutcome();

  factory ImportOutcome.fromJson(Map<String, dynamic> json) {
    return switch (reqString(json, 'outcome')) {
      'imported' => const Imported(),
      'replace_requested' => ReplaceRequested(
        currentHost: reqString(json, 'current_host'),
        newHost: reqString(json, 'new_host'),
      ),
      final other => throw FormatException('итог импорта «$other» неизвестен'),
    };
  }
}

final class Imported extends ImportOutcome {
  const Imported();
  @override
  List<Object?> get props => const [];
}

final class ReplaceRequested extends ImportOutcome {
  const ReplaceRequested({required this.currentHost, required this.newHost});
  final String currentHost;
  final String newHost;
  @override
  List<Object?> get props => [currentHost, newHost];
}

/// Итог обновления подписки.
sealed class RefreshOutcome with Value {
  const RefreshOutcome();

  factory RefreshOutcome.fromJson(Map<String, dynamic> json) {
    return switch (reqString(json, 'outcome')) {
      'updated' => Updated(reqInt(json, 'version')),
      'not_modified' => const NotModified(),
      final other => throw FormatException(
        'итог обновления «$other» неизвестен',
      ),
    };
  }
}

final class Updated extends RefreshOutcome {
  const Updated(this.version);
  final int version;
  @override
  List<Object?> get props => [version];
}

final class NotModified extends RefreshOutcome {
  const NotModified();
  @override
  List<Object?> get props => const [];
}

/// Результат «Проверить связь» по одному движку.
class EngineProbeReport with Value {
  const EngineProbeReport({
    required this.engine,
    required this.state,
    this.rttMs,
    this.lossPct = 0,
    this.handshakeAgeS,
  });

  final String engine;
  final EngineStatus state;
  final int? rttMs;
  final int lossPct;
  final int? handshakeAgeS;

  factory EngineProbeReport.fromJson(Map<String, dynamic> json) {
    return EngineProbeReport(
      engine: reqString(json, 'engine'),
      state: EngineStatus.parse(reqString(json, 'state')),
      rttMs: optInt(json, 'rtt_ms'),
      lossPct: reqInt(json, 'loss_pct'),
      handshakeAgeS: optInt(json, 'handshake_age_s'),
    );
  }

  @override
  List<Object?> get props => [engine, state, rttMs, lossPct, handshakeAgeS];
}
