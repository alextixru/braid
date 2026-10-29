/// Проверка ввода на экране «Движки → CSQTT» (ист. 14, 15): ссылка на звонок
/// VK (`https://vk.com/call/join/<хеш>`) или голый хеш.
///
/// Правило повторяет `parse_call_hash` из `braid-engine-csqtt` один в один:
/// экран не должен принять то, что ядро отвергнет, и наоборот. Ядро всё равно
/// проверяет ввод само (`UserInput::VkHashes`), но ошибка «на месте», до
/// отправки, не требует ни запущенного движка, ни сессии.
library;

/// Текст ошибки для экрана (ист. 15); тот же, что отдаёт ядро.
const String kInvalidCallLink = 'Это не ссылка на звонок VK';

const String _joinPath = '/call/join/';
const int _minHashLength = 16;
const int _maxHashLength = 128;

final RegExp _hashChars = RegExp(r'^[A-Za-z0-9_-]+$');
const String _wrappers = '<>"\'';

/// Хеш звонка из ссылки или голого хеша; `null` — это не ссылка на звонок VK.
String? parseCallHash(String input) {
  final value = _trimAny(input.trim(), _wrappers).trim();
  if (value.isEmpty) return null;
  final lower = _asciiLower(value);
  final index = lower.indexOf(_joinPath);
  if (index >= 0) {
    final host = _withoutScheme(lower.substring(0, index));
    if (!_isVkHost(host)) return null;
    final tail = value.substring(index + _joinPath.length);
    var end = tail.length;
    for (final stop in const ['?', '#', '/']) {
      final at = tail.indexOf(stop);
      if (at >= 0 && at < end) end = at;
    }
    return _hashIfValid(tail.substring(0, end));
  }
  if (lower.contains('://') || lower.contains('/')) return null;
  return _hashIfValid(value);
}

/// Хеш для списка: хеш — секрет звонка, целиком его на экране не держим.
String maskHash(String hash) {
  if (hash.length <= 12) return hash;
  return '${hash.substring(0, 6)}…${hash.substring(hash.length - 5)}';
}

String? _hashIfValid(String hash) {
  final validLength =
      hash.length >= _minHashLength && hash.length <= _maxHashLength;
  return validLength && _hashChars.hasMatch(hash) ? hash : null;
}

bool _isVkHost(String host) {
  final withoutUser = host.split('@').last;
  final name = withoutUser.split(':').first;
  return name == 'vk.com' ||
      name == 'vk.ru' ||
      name.endsWith('.vk.com') ||
      name.endsWith('.vk.ru');
}

String _withoutScheme(String host) {
  var rest = host;
  while (rest.startsWith('https://')) {
    rest = rest.substring('https://'.length);
  }
  while (rest.startsWith('http://')) {
    rest = rest.substring('http://'.length);
  }
  return rest;
}

/// Только ASCII, как `to_ascii_lowercase` в ядре: длина строки не меняется,
/// и позиция, найденная в копии, годится для исходной строки.
String _asciiLower(String text) {
  final out = StringBuffer();
  for (final unit in text.codeUnits) {
    out.writeCharCode(unit >= 0x41 && unit <= 0x5A ? unit + 0x20 : unit);
  }
  return out.toString();
}

String _trimAny(String text, String chars) {
  var start = 0;
  var end = text.length;
  while (start < end && chars.contains(text[start])) {
    start++;
  }
  while (end > start && chars.contains(text[end - 1])) {
    end--;
  }
  return text.substring(start, end);
}
