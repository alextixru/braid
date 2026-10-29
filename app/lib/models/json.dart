/// Разбор JSON-документов ядра. Документ, который не читается, — это
/// `FormatException` с именем поля: приложение показывает ошибку, а не
/// падает на неожиданной форме.
library;

Map<String, dynamic> asObject(Object? value, String what) {
  if (value is Map<String, dynamic>) return value;
  throw FormatException('$what: ожидался объект');
}

List<Object?> asList(Object? value, String what) {
  if (value is List<Object?>) return value;
  throw FormatException('$what: ожидался список');
}

String reqString(Map<String, dynamic> json, String key) {
  final value = json[key];
  if (value is String) return value;
  throw FormatException('поле «$key»: ожидалась строка');
}

String? optString(Map<String, dynamic> json, String key) {
  final value = json[key];
  if (value == null) return null;
  if (value is String) return value;
  throw FormatException('поле «$key»: ожидалась строка');
}

int reqInt(Map<String, dynamic> json, String key) {
  final value = json[key];
  if (value is int) return value;
  throw FormatException('поле «$key»: ожидалось целое число');
}

int? optInt(Map<String, dynamic> json, String key) {
  final value = json[key];
  if (value == null) return null;
  if (value is int) return value;
  throw FormatException('поле «$key»: ожидалось целое число');
}

bool reqBool(Map<String, dynamic> json, String key) {
  final value = json[key];
  if (value is bool) return value;
  throw FormatException('поле «$key»: ожидалось да/нет');
}

List<String> reqStrings(Map<String, dynamic> json, String key) {
  final value = json[key];
  if (value is List<Object?> && value.every((e) => e is String)) {
    return List<String>.unmodifiable(value.cast<String>());
  }
  throw FormatException('поле «$key»: ожидался список строк');
}

DateTime reqTime(Map<String, dynamic> json, String key) {
  final text = reqString(json, key);
  final parsed = DateTime.tryParse(text);
  if (parsed == null) throw FormatException('поле «$key»: не дата — $text');
  return parsed;
}

DateTime? optTime(Map<String, dynamic> json, String key) {
  if (json[key] == null) return null;
  return reqTime(json, key);
}
