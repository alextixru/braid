/// Перенос своих правил файлом (десктоп, ист. 52): домены и подсети в JSON.
/// Приложения не переносятся — они бывают только на Android, где работает
/// системная резервная копия.
library;

import 'dart:convert';

import 'package:braid/util/rule_input.dart';

/// Что нашлось в файле: принятое в каноническом виде и отброшенное как есть.
class RulesImportPlan {
  const RulesImportPlan({
    required this.domains,
    required this.subnets,
    required this.rejected,
  });

  final List<String> domains;
  final List<String> subnets;

  /// Записи, которые не домен и не подсеть: пользователь увидит их число.
  final List<String> rejected;
}

abstract final class RulesTransfer {
  static const format = 'braid-rules';
  static const version = 1;

  /// Имя файла по умолчанию в диалоге сохранения.
  static const fileName = 'braid-rules.json';

  static String encode({
    required List<String> domains,
    required List<String> subnets,
  }) {
    return const JsonEncoder.withIndent('  ').convert({
      'format': format,
      'version': version,
      'domains': domains,
      'subnets': subnets,
    });
  }

  /// Разобрать файл. Не тот файл или другая версия — [FormatException] с
  /// готовым текстом; отдельные плохие записи не валят импорт, а попадают в
  /// [RulesImportPlan.rejected].
  static RulesImportPlan decode(String text) {
    final Object? json;
    try {
      json = jsonDecode(text);
    } on FormatException {
      throw const FormatException('Это не файл правил Braid');
    }
    if (json is! Map<String, dynamic> || json['format'] != format) {
      throw const FormatException('Это не файл правил Braid');
    }
    if (json['version'] != version) {
      throw const FormatException('Файл правил другой версии — обнови Braid');
    }
    final rejected = <String>[];
    final domains = <String>[];
    final subnets = <String>[];
    for (final entry in _strings(json['domains'])) {
      try {
        _add(domains, RuleInput.normalizeDomain(entry));
      } on RuleInputException {
        rejected.add(entry);
      }
    }
    for (final entry in _strings(json['subnets'])) {
      try {
        _add(subnets, RuleInput.normalizeCidr(entry));
      } on RuleInputException {
        rejected.add(entry);
      }
    }
    return RulesImportPlan(
      domains: domains,
      subnets: subnets,
      rejected: rejected,
    );
  }

  static Iterable<String> _strings(Object? value) {
    if (value == null) return const [];
    if (value is! List) throw const FormatException('Это не файл правил Braid');
    return value.map((e) => e is String ? e : '$e');
  }

  static void _add(List<String> into, String value) {
    if (!into.contains(value)) into.add(value);
  }
}
