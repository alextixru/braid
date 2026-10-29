/// Ввод своих правил: домен или подсеть IPv4. Приводит ввод к той форме, которую
/// принимает ядро, и объясняет ошибку по-русски. Правила те же, что у ядра
/// (`braid_core::rules::normalize_domain`, `Ipv4Net`): эталонные случаи
/// в `test/fixtures/rule_input_cases.json` проверяет и Rust (`contract.rs`).
library;

enum RuleKind { domain, subnet }

/// Ввод приведён к канонической форме.
class NormalizedRule {
  const NormalizedRule(this.kind, this.value);

  final RuleKind kind;
  final String value;

  @override
  bool operator ==(Object other) =>
      other is NormalizedRule && other.kind == kind && other.value == value;

  @override
  int get hashCode => Object.hash(kind, value);

  @override
  String toString() => '$kind $value';
}

/// Ввод не принят; [message] можно показывать как есть.
class RuleInputException implements Exception {
  const RuleInputException(this.message);

  final String message;

  @override
  String toString() => message;
}

abstract final class RuleInput {
  static const _empty = RuleInputException('Введи домен или подсеть');
  static const _notDomain = RuleInputException(
    'Это не домен — нужно имя вроде example.com или *.example.com',
  );
  static const _ipAsDomain = RuleInputException(
    'Это IP-адрес, а не домен — добавь его как подсеть, например 1.2.3.4/32',
  );
  static const _notAscii = RuleInputException(
    'Домен только латиницей; кириллический впиши в виде xn--…',
  );
  static const _notSubnet = RuleInputException(
    'Это не подсеть IPv4 — нужно, например, 10.0.0.0/8 или 1.2.3.4',
  );
  static const _neither = RuleInputException(
    'Не похоже ни на домен, ни на подсеть IPv4',
  );

  /// `https://x.y/путь` → `x.y`; порт и `user@` отбрасываются; `*.x.y` остаётся.
  static String normalizeDomain(String input) {
    var s = input.trim();
    final scheme = s.indexOf('://');
    if (scheme >= 0) s = s.substring(scheme + 3);
    final end = s.indexOf(RegExp(r'[/?#]'));
    if (end >= 0) s = s.substring(0, end);
    final at = s.lastIndexOf('@');
    if (at >= 0) s = s.substring(at + 1);
    final colon = s.lastIndexOf(':');
    if (colon >= 0) {
      final port = s.substring(colon + 1);
      if (port.isNotEmpty && RegExp(r'^[0-9]+$').hasMatch(port)) {
        s = s.substring(0, colon);
      }
    }
    if (s.isEmpty) throw _empty;
    if (_parseIpv4(s) != null) throw _ipAsDomain;
    return _parsePattern(s, input.trim());
  }

  /// `10.1.2.3/8` → `10.0.0.0/8`; адрес без длины — `/32`.
  static String normalizeCidr(String input) {
    final s = input.trim();
    if (s.isEmpty) throw _empty;
    final slash = s.indexOf('/');
    final address = _parseIpv4(slash < 0 ? s : s.substring(0, slash));
    if (address == null) throw _notSubnet;
    var prefix = 32;
    if (slash >= 0) {
      final text = s.substring(slash + 1);
      if (!RegExp(r'^[0-9]{1,2}$').hasMatch(text)) throw _notSubnet;
      prefix = int.parse(text);
      if (prefix > 32) throw _notSubnet;
    }
    final value =
        (address[0] << 24) |
        (address[1] << 16) |
        (address[2] << 8) |
        address[3];
    final mask = prefix == 0 ? 0 : (0xFFFFFFFF << (32 - prefix)) & 0xFFFFFFFF;
    final network = value & mask;
    return '${(network >> 24) & 255}.${(network >> 16) & 255}.'
        '${(network >> 8) & 255}.${network & 255}/$prefix';
  }

  /// Одно поле для обоих видов: IPv4 (с длиной или без) — подсеть, остальное — домен.
  static NormalizedRule classify(String input) {
    final s = input.trim();
    if (s.isEmpty) throw _empty;
    if (RegExp(r'^[0-9]{1,3}(\.[0-9]{1,3}){3}(/[0-9]+)?$').hasMatch(s)) {
      return NormalizedRule(RuleKind.subnet, normalizeCidr(s));
    }
    // Слова через пробел — это не домен ни на каком языке.
    if (RegExp(r'\s').hasMatch(s)) throw _neither;
    try {
      return NormalizedRule(RuleKind.domain, normalizeDomain(s));
    } on RuleInputException catch (e) {
      // В общем поле «не домен» значит «и не подсеть тоже» — отвечаем за оба вида.
      if (identical(e, _notDomain)) throw _neither;
      rethrow;
    }
  }

  /// Домен из текста буфера обмена: ссылка или «голый» домен; `www.` отбрасывается
  /// (свой домен без `*.` и так действует на поддомены). Нет домена — `null`.
  static String? domainFromText(String text) {
    for (final raw in text.split(RegExp(r'\s+'))) {
      final token = raw.replaceAll(
        RegExp(r'^[(\[<"\u00ab\x27]+|[)\]>,;:!?"\u00bb\x27]+$'),
        '',
      );
      if (token.isEmpty) continue;
      final String domain;
      try {
        domain = normalizeDomain(token);
      } on RuleInputException {
        continue;
      }
      final base = domain.startsWith('*.') ? domain.substring(2) : domain;
      final tld = base.substring(base.lastIndexOf('.') + 1);
      // «e.g» и «file.7z» — не домены: верхний уровень — буквы, от двух.
      if (!RegExp(r'^([a-z]{2,}|xn--[a-z0-9-]+)$').hasMatch(tld)) continue;
      return domain.startsWith('www.') ? domain.substring(4) : domain;
    }
    return null;
  }

  /// Четыре октета строго, как в Rust: без плюсов и ведущих нулей.
  static List<int>? _parseIpv4(String text) {
    final parts = text.split('.');
    if (parts.length != 4) return null;
    final octets = <int>[];
    for (final part in parts) {
      if (!RegExp(r'^(0|[1-9][0-9]{0,2})$').hasMatch(part)) return null;
      final value = int.parse(part);
      if (value > 255) return null;
      octets.add(value);
    }
    return octets;
  }

  /// `DomainPattern::parse`: метки `[a-z0-9_-]`, ведущие `*.`, минимум две
  /// «настоящие» метки; регистр понижается только у ASCII.
  static String _parsePattern(String text, String original) {
    final trimmed = text.trim();
    if (trimmed.isEmpty) throw _empty;
    var end = trimmed.length;
    while (end > 0 && trimmed.codeUnitAt(end - 1) == 0x2E) {
      end--;
    }
    final name = String.fromCharCodes(
      trimmed
          .substring(0, end)
          .codeUnits
          .map((c) => c >= 0x41 && c <= 0x5A ? c + 32 : c),
    );
    if (name.length > 253) throw _notDomain;
    if (name.codeUnits.any((c) => c > 0x7F)) throw _notAscii;
    var baseStarted = false;
    var baseLabels = 0;
    for (final label in name.split('.')) {
      if (label == '*') {
        if (baseStarted) throw _notDomain;
        continue;
      }
      baseStarted = true;
      baseLabels++;
      if (label.isEmpty || label.length > 63) throw _notDomain;
      if (!RegExp(r'^[a-z0-9_-]+$').hasMatch(label)) throw _notDomain;
      if (label.startsWith('-') || label.endsWith('-')) throw _notDomain;
    }
    if (baseLabels < 2) throw _notDomain;
    return name;
  }
}
